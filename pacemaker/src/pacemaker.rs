use std::{
    collections::VecDeque,
    io,
    sync::{atomic::AtomicBool, Arc},
};

use crate::{
    data::{PeerEvent, SyncStatus},
    timer::{DefaultTimer, TimeoutEvent},
};
use crate::{liveness_storage::LivenessStorage, network::NetworkAdaptor};
use hotstuff_rs::{
    data::ViewNumber,
    safety::{
        machine::{self, Safety},
        safety_storage::in_mem::InMemoryStorage,
    },
};

use log::{error, info};
use machine::SafetyEvent;

pub type TchanR<T> = tokio::sync::mpsc::Receiver<T>;
pub type TchanS<T> = tokio::sync::mpsc::Sender<T>;
pub type PeerEventRecvr = TchanR<PeerEvent>;
pub type PeerEventSender = TchanS<PeerEvent>;
pub type CtlRecvr = tokio::sync::broadcast::Receiver<()>;
pub type CtlSender = tokio::sync::broadcast::Sender<()>;

pub struct Pacemaker<S: LivenessStorage + Send + Sync> {
    // pacemaker identifier
    view: ViewNumber,
    // id: ReplicaID,
    elector: crate::elector::RoundRobinLeaderElector,
    // LivenessStorage implementation. May be wrapped by Arc
    storage: S,

    sync_state: Arc<AtomicBool>,
    pending_prop: VecDeque<SafetyEvent>,

    machine_adaptor: AsyncMachineAdaptor,
    net_adaptor: NetworkAdaptor,

    timer: DefaultTimer,
    timeout_ch: TchanR<TimeoutEvent>,
}

/// Leader elector using round-robin algorithm.
impl<S: LivenessStorage + Send + Sync> Pacemaker<S> {
    pub async fn run(&mut self, mut quit_ch: TchanR<()>) -> io::Result<()> {
        info!("pacemaker up");
        let mut quit = false;

        loop {
            tokio::select! {
                Some(()) = quit_ch.recv() => {
                    quit = true;
                },
                Some(ready) = self.machine_adaptor.ready().recv() => {
                    self.process_ready_event(ready).await?;
                },
                Some(net_event) = self.net_adaptor.event_recvr.recv() => {
                    self.process_network_event(net_event).await?;
                },
                Some(te) = self.timeout_ch.recv() => {
                    self.process_timeout_event(te).await?;
                },
            }
            if quit {
                break;
            }
        }
        info!("pacemaker down");
        Ok(())
    }

    async fn process_timeout_event(&mut self, te: TimeoutEvent) -> io::Result<()> {
        match te {
            TimeoutEvent::ViewTimeout(view) => {
                if view >= self.view {
                    self.timer.stop_view_timer();
                    self.goto_new_view(view);
                    self.timer.start(
                        self.timer.timeout_by_delay(),
                        TimeoutEvent::ViewTimeout(self.view),
                    );
                }
            }
            TimeoutEvent::BranchSyncTimeout(lower, upper) => {
                // check pending_prop
                self.check_pending_prop(self.storage.get_qc_high().height())
                    .await?
            }
        }

        Ok(())
    }

    // todo
    async fn process_network_event(&mut self, net_event: PeerEvent) -> io::Result<()> {
        match net_event {
            PeerEvent::NewProposal { ctx, prop } => {
                if self.storage.is_qc_node_exists(prop.justify()) {
                    self.emit_safety_event(SafetyEvent::RecvProposal(ctx, Arc::new(*prop)))
                        .await?;
                } else {
                    self.pending_prop
                        .push_back(SafetyEvent::RecvProposal(ctx, Arc::new(*prop)));
                }
            }
            PeerEvent::AcceptProposal { ctx, prop, sign } => {
                if let Some(sign) = sign {
                    self.emit_safety_event(SafetyEvent::RecvSign(ctx, Arc::new(*prop), sign))
                        .await?;
                }
            }
            PeerEvent::Timeout { ctx, tc } => {
                info!("recv timeout msg view={} from {}", ctx.view, &ctx.from);
                match self.storage.append_tc(&tc) {
                    Ok(v) => {
                        if v > self.view {
                            // view=self.view timeouts right now.
                            // self.view will increase once the pacemaker recv local timeout event.
                            self.timer.view_timeout(self.view);
                        }
                    }
                    Err(e) => {
                        error!("{:?}", e);
                    }
                }
            }
            PeerEvent::BranchSyncRequest { ctx, strategy } => {
                match self.storage.fetch_branch(&strategy) {
                    Ok(bd) => {
                        self.emit_peer_event(PeerEvent::BranchSyncResponse {
                            ctx,
                            strategy,
                            branch: Some(bd),
                            status: SyncStatus::Success,
                        })
                        .await?;
                    }
                    Err(e) => {
                        error!("{:?}", e);
                        self.emit_peer_event(PeerEvent::BranchSyncResponse {
                            ctx,
                            strategy,
                            branch: None,
                            status: SyncStatus::ProposalNonExists,
                        })
                        .await?;
                    }
                }
            }
            PeerEvent::BranchSyncResponse {
                ctx,
                status,
                branch,
                ..
            } => match status {
                SyncStatus::Success if branch.is_some() => {
                    self.emit_safety_event(SafetyEvent::BranchSync(ctx, branch.unwrap().data))
                        .await?
                }
                other => error!("{:?}", other),
            },
            _ => {}
        }
        Ok(())
    }

    async fn process_ready_event(&mut self, ready: machine::Ready) -> io::Result<()> {
        match ready {
            machine::Ready::Nil => {}
            machine::Ready::InternalState(_, _) => {}
            machine::Ready::NewProposal(ctx, prop) => {
                self.emit_peer_event(PeerEvent::NewProposal {
                    ctx,
                    prop: Box::new(prop.as_ref().clone()),
                })
                .await?;
            }
            machine::Ready::UpdateQCHigh(_, node) => {
                if let Err(e) = self.storage.update_qc_high(node.justify()) {
                    error!("{:?}", e);
                }
                // check pending
                self.check_pending_prop(node.height()).await?
            }
            machine::Ready::Signature(ctx, prop, sign) => {
                self.emit_peer_event(PeerEvent::AcceptProposal {
                    ctx,
                    prop: Box::new(prop.as_ref().clone()),
                    sign: Some(sign),
                })
                .await?;
            }
            machine::Ready::CommitState(_, _) => {}
        }
        Ok(())
    }

    async fn check_pending_prop(&mut self, node_height: ViewNumber) -> io::Result<()> {
        if let Some(SafetyEvent::RecvProposal(ctx, pending_new_prop)) =
            self.pending_prop.pop_front()
        {
            if pending_new_prop.justify().view() == node_height {
                self.emit_safety_event(SafetyEvent::RecvProposal(ctx, pending_new_prop))
                    .await?;
            } else {
                // re-push
                self.pending_prop
                    .push_front(SafetyEvent::RecvProposal(ctx, pending_new_prop));
            }
        }
        Ok(())
    }

    #[inline]
    async fn emit_safety_event(&mut self, se: SafetyEvent) -> io::Result<()> {
        self.machine_adaptor.safety().send(se).await.unwrap();
        Ok(())
    }

    // todo: consider use Pacemaker Error
    async fn emit_peer_event(&mut self, pe: PeerEvent) -> io::Result<()> {
        let dur = self.timer.timeout_by_delay();
        if let Err(_) = self.net_adaptor.event_sender.send_timeout(pe, dur).await {
            error!("pacemaker -> network adaptor block and timeout");
        }

        Ok(())
    }

    fn update_pm_status(&mut self, view: ViewNumber) {
        self.view = ViewNumber::max(self.view, view) + 1;
    }

    fn goto_new_view(&mut self, view: ViewNumber) {
        self.update_pm_status(view);
        self.elector.view_change(view);
    }
}

pub struct AsyncMachineAdaptor {
    pub ready_recvr: TchanR<machine::Ready>,
    pub safety_sender: TchanS<machine::SafetyEvent>,
}

impl AsyncMachineAdaptor {
    #[inline]
    pub fn ready(&mut self) -> &mut TchanR<machine::Ready> {
        &mut self.ready_recvr
    }

    #[inline]
    pub fn safety(&mut self) -> &mut TchanS<machine::SafetyEvent> {
        &mut self.safety_sender
    }
}

/// Async wrapper for `Machine`.
pub struct AsyncMachineWrapper {
    // buffer.
    safety_in: TchanR<machine::SafetyEvent>,
    ready_out: TchanS<machine::Ready>,
    machine: machine::Machine<InMemoryStorage>,
}

impl AsyncMachineWrapper {
    pub async fn run(&mut self, mut quit_ch: CtlRecvr) -> io::Result<()> {
        info!("machine up");
        let mut quit = false;
        while !quit {
            tokio::select! {
                _ = quit_ch.recv() => {
                    quit = true;
                },
                Some(se) = self.safety_in.recv() => {
                    match self.machine.process_safety_event(se){
                        Ok(rd) => {
                            if let machine::Ready::Nil = &rd{
                                continue;
                            }
                            if let Err(e) = self.ready_out.send(rd).await{
                                // Receiver half is closed.
                                quit = true;
                                error!("{}", e.to_string());
                            }
                        },
                        Err(e) => {
                            error!("{}", e.to_string());
                        }
                    }
                },
            }
        }
        info!("machine down");
        Ok(())
    }
}
