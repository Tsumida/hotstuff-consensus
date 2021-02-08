use std::{io, sync::Arc, unimplemented};

use crate::liveness_storage::LivenessStorage;
use crate::{data::PeerEvent, timer::DefaultTimer};
use hotstuff_rs::{
    data::{ReplicaID, ViewNumber},
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

pub struct Pacemaker {
    // pacemaker identifier
    view: ViewNumber,
    id: ReplicaID,

    // todo: consider use trait object
    elector: crate::elector::RoundRobinLeaderElector,
    storage: Box<dyn LivenessStorage + Send + Sync>,

    leader: Option<ReplicaID>,

    machine_adaptor: AsyncMachineAdaptor,

    net_sender: PeerEventSender,
    net_recvr: PeerEventRecvr,

    timer: DefaultTimer,
    timeout_ch: TchanR<ViewNumber>,
}

/// Leader elector using round-robin algorithm.
impl Pacemaker {
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
                Some(net_event) = self.net_recvr.recv() => {
                    self.process_network_event(net_event).await?;
                },
                Some(view) = self.timeout_ch.recv() => {
                    if view >= self.view{
                        self.timer.stop_all_timer();
                        self.goto_new_view(view);
                        self.timer.start(self.view, self.timer.timeout_by_delay());
                    }
                },
            }
            if quit {
                break;
            }
        }
        info!("pacemaker down");
        Ok(())
    }

    // todo
    async fn process_network_event(&mut self, net_event: PeerEvent) -> io::Result<()> {
        match net_event {
            PeerEvent::NewProposal { ctx, prop } => {
                self.emit_safety_event(SafetyEvent::RecvProposal(ctx, Arc::new(*prop)))
                    .await?;
            }
            PeerEvent::AcceptProposal { ctx, prop, sign } => {
                if let Some(sign) = sign {
                    self.emit_safety_event(SafetyEvent::RecvSign(ctx, Arc::new(*prop), sign))
                        .await?;
                }
            }
            PeerEvent::Timeout { ctx, tc } => {
                info!("recv timeout msg view={} from {}", ctx.view, &ctx.from);
                let _ = self.storage.append_tc(&tc);
            }
            PeerEvent::BranchSyncRequest { ctx, strategy } => unimplemented!(),
            PeerEvent::BranchSyncResponse { ctx, branch } => unimplemented!(),
            _ => {}
        }
        Ok(())
    }

    // todo
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
                self.storage.update_qc_high(node.justify());
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

    #[inline]
    async fn emit_safety_event(&mut self, se: SafetyEvent) -> io::Result<()> {
        self.machine_adaptor.safety().send(se).await.unwrap();
        Ok(())
    }

    // todo: consider use Pacemaker Error
    async fn emit_peer_event(&mut self, pe: PeerEvent) -> io::Result<()> {
        let dur = self.timer.timeout_by_delay();
        if let Err(e) = self.net_sender.send_timeout(pe, dur).await {
            error!("pacemaker -> network adaptor block and timeout");
        }

        Ok(())
    }

    fn update_pm_status(&mut self, view: ViewNumber) {
        self.view = ViewNumber::max(self.view, view) + 1;
        self.update_leader();
    }

    fn update_leader(&mut self) {
        self.leader = Some(self.elector.get_leader(self.view).clone());
    }

    fn goto_new_view(&mut self, view: ViewNumber) {
        self.update_pm_status(view);
    }
}

pub struct AsyncMachineAdaptor {
    pub ready_in: TchanR<machine::Ready>,
    pub safety_out: TchanS<machine::SafetyEvent>,
}

impl AsyncMachineAdaptor {
    #[inline]
    pub fn ready(&mut self) -> &mut TchanR<machine::Ready> {
        &mut self.ready_in
    }

    #[inline]
    pub fn safety(&mut self) -> &mut TchanS<machine::SafetyEvent> {
        &mut self.safety_out
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
    pub async fn run(
        &mut self,
        mut quit_ch: tokio::sync::broadcast::Receiver<()>,
    ) -> io::Result<()> {
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
