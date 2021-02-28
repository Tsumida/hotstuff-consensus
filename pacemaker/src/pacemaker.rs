use crate::{
    data::{BranchSyncStrategy, PeerEvent, SyncStatus},
    timer::{DefaultTimer, TimeoutEvent},
};
use crate::{liveness_storage::LivenessStorage, network::NetworkAdaptor};
use hotstuff_rs::safety::machine::{self, Safety, SafetyEvent, SafetyStorage};
use hs_data::{msg::Context, ReplicaID, ViewNumber};
use std::{
    collections::BinaryHeap,
    io,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
};

use log::{error, info};

pub type TchanR<T> = tokio::sync::mpsc::Receiver<T>;
pub type TchanS<T> = tokio::sync::mpsc::Sender<T>;
pub type PeerEventRecvr = TchanR<PeerEvent>;
pub type PeerEventSender = TchanS<PeerEvent>;
pub type CtlRecvr = tokio::sync::broadcast::Receiver<()>;
pub type CtlSender = tokio::sync::broadcast::Sender<()>;

const SYNC_SYNCHRONIZING: u8 = 0;
const SYNC_IDLE: u8 = 1;

static BATCH_SIZE: usize = 8;

// TODO
#[derive(Debug, Clone)]
pub(crate) struct SafetyEventWrapper {
    pub se: SafetyEvent,
    pub view: i64,
}

impl Eq for SafetyEventWrapper {}

impl Ord for SafetyEventWrapper {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.view.cmp(&other.view)
    }
}

impl PartialOrd for SafetyEventWrapper {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for SafetyEventWrapper {
    fn eq(&self, other: &Self) -> bool {
        self.view == other.view
    }
}

impl SafetyEventWrapper {
    fn from(view: ViewNumber, se: SafetyEvent) -> Self {
        Self {
            view: -1 * view as i64,
            se,
        }
    }
}

impl Into<SafetyEvent> for SafetyEventWrapper {
    fn into(self) -> SafetyEvent {
        self.se
    }
}

#[test]
fn test_pending_prop() {
    use std::mem::MaybeUninit;
    let mut pending_prop: BinaryHeap<SafetyEventWrapper> = BinaryHeap::with_capacity(3);
    pending_prop.push(SafetyEventWrapper::from(0, unsafe {
        MaybeUninit::uninit().assume_init()
    }));
    pending_prop.push(SafetyEventWrapper::from(1, unsafe {
        MaybeUninit::uninit().assume_init()
    }));
    pending_prop.push(SafetyEventWrapper::from(2, unsafe {
        MaybeUninit::uninit().assume_init()
    }));

    assert!(0 == pending_prop.pop().unwrap().view);
    assert!(-1 == pending_prop.pop().unwrap().view);
    assert!(-2 == pending_prop.pop().unwrap().view);
}

pub struct Pacemaker<S: LivenessStorage + Send + Sync> {
    // pacemaker identifier
    view: ViewNumber,
    id: ReplicaID,
    elector: crate::elector::RoundRobinLeaderElector,
    // LivenessStorage implementation. May be wrapped by Arc
    storage: S,

    sync_state: Arc<AtomicU8>,
    pending_prop: BinaryHeap<SafetyEventWrapper>,

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
                    self.goto_new_view(view);
                    self.timer.start(
                        self.timer.timeout_by_delay(),
                        TimeoutEvent::ViewTimeout(self.view),
                    );

                    // check pending_prop
                    if self
                        .sync_state
                        .compare_exchange(
                            SYNC_SYNCHRONIZING,
                            SYNC_IDLE,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        )
                        .is_ok()
                    {
                        self.check_pending_prop(self.storage.get_leaf().height())
                            .await?
                    }
                }
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
                    self.pending_prop.push(SafetyEventWrapper::from(
                        prop.height(),
                        SafetyEvent::RecvProposal(ctx, Arc::new(*prop)),
                    ));
                    self.check_pending_prop(self.storage.get_leaf().height())
                        .await?;
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
                match self.storage.append_tc(tc) {
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
            machine::Ready::BranchSyncDone(leaf) => {
                // BranchSync
                if self
                    .sync_state
                    .compare_exchange(
                        SYNC_SYNCHRONIZING,
                        SYNC_IDLE,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
                    .is_ok()
                {
                    info!("branch sync done, leaf={}", leaf.height());
                    self.check_pending_prop(leaf.height()).await?;
                }
            }
        }
        Ok(())
    }

    /// Check pending queue, start branch synchronization if there are proposals lack of `justify.node` .
    async fn check_pending_prop(&mut self, leaf_height: ViewNumber) -> io::Result<()> {
        if let Some(sw) = self.pending_prop.pop() {
            let view = -sw.view as ViewNumber;
            if view <= leaf_height {
                self.emit_safety_event(sw.se).await?;
            } else {
                // re-push
                self.pending_prop.push(sw);
                // start synchronizing
                if self
                    .sync_state
                    .compare_exchange(
                        SYNC_IDLE,
                        SYNC_SYNCHRONIZING,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
                    .is_ok()
                {
                    let grow_from = self.storage.get_locked_node().height();

                    info!(
                        "start branch sync: from={}, batch-size={}]",
                        grow_from, BATCH_SIZE
                    );
                    self.emit_peer_event(PeerEvent::BranchSyncRequest {
                        ctx: Context {
                            from: self.id.clone(),
                            view: self.view,
                        },
                        strategy: BranchSyncStrategy::Grow {
                            grow_from,
                            batch_size: BATCH_SIZE,
                        },
                    })
                    .await?;
                }
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
pub struct AsyncMachineWrapper<S>
where
    S: SafetyStorage,
{
    // buffer.
    safety_in: TchanR<machine::SafetyEvent>,
    ready_out: TchanS<machine::Ready>,
    machine: machine::Machine<S>,
}

impl<S: SafetyStorage> AsyncMachineWrapper<S> {
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
