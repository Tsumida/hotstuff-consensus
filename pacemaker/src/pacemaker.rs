use std::{io, ops::Sub, unimplemented};

use crate::{data::PeerEvent, timer::DefaultTimer};
// use hotstuff_rs;
use crate::liveness_storage::LivenessStorage;
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

pub struct Pacemaker {
    // pacemaker identifier
    view: ViewNumber,
    id: ReplicaID,
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
                        self.timer.stop_all_prev_timer();
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
                self.emit_safety_event(SafetyEvent::RecvProposal(ctx, prop))
                    .await?;
            }
            PeerEvent::AcceptProposal { ctx, prop, sign } => {
                if let Some(sign) = sign {
                    self.emit_safety_event(SafetyEvent::RecvSign(ctx, prop, sign))
                        .await?;
                }
            }
            PeerEvent::Timeout { ctx, tc } => unimplemented!(),
            PeerEvent::BranchSyncRequest { ctx, strategy } => unimplemented!(),
            PeerEvent::BranchSyncResponse { ctx, branch } => unimplemented!(),
        }
        Ok(())
    }

    // todo
    async fn process_ready_event(&mut self, ready: machine::Ready) -> io::Result<()> {
        match ready {
            machine::Ready::Nil => {}
            machine::Ready::InternalState(_, _) => {}
            machine::Ready::NewProposal(ctx, prop) => {
                self.emit_peer_event(PeerEvent::NewProposal { ctx, prop })
                    .await?;
            }
            machine::Ready::UpdateQCHigh(_, node) => {
                self.storage.update_qc_high(node.justify());
            }
            machine::Ready::Signature(_, _, _) => {}
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
    quit_ch: TchanR<()>,

    // buffer.
    safety_in: TchanR<machine::SafetyEvent>,
    ready_out: TchanS<machine::Ready>,
    machine: machine::Machine<InMemoryStorage>,
}

impl AsyncMachineWrapper {
    pub async fn run(&mut self) -> io::Result<()> {
        info!("machine up");
        let mut quit = false;
        while !quit {
            tokio::select! {
                Some(()) = self.quit_ch.recv() => {
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
