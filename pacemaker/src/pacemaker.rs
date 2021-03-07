use crate::{
    data::{BranchSyncStrategy, PeerEvent, SyncStatus},
    elector::RoundRobinLeaderElector,
    timer::{DefaultTimer, TimeoutEvent},
};
use crate::{liveness_storage::LivenessStorage, network::NetworkAdaptor};
use hotstuff_rs::safety::machine::{self, Machine, Ready, Safety, SafetyEvent, SafetyStorage};
use hs_data::{msg::Context, ReplicaID, ViewNumber};
use std::{
    collections::{BinaryHeap, VecDeque},
    io,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    time::Duration,
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

const DEFAULT_FLUSH_INTERVAL: u64 = 500;
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

// refactor: Separate SafetyStorage & LivenessStorage
pub struct Pacemaker<S>
where
    S: SafetyStorage + LivenessStorage + Send + Sync,
{
    // pacemaker identifier
    view: ViewNumber,
    id: ReplicaID,
    pub elector: crate::elector::RoundRobinLeaderElector,

    machine: Machine<S>,
    ready_queue: VecDeque<Ready>,
    pending_prop: BinaryHeap<SafetyEventWrapper>,

    pub sync_state: Arc<AtomicU8>,

    net_adaptor: NetworkAdaptor,

    timer: DefaultTimer,
    timeout_ch: TchanR<TimeoutEvent>,
}

/// Leader elector using round-robin algorithm.
impl<S> Pacemaker<S>
where
    S: SafetyStorage + LivenessStorage + Send + Sync,
{
    pub fn new(
        id: ReplicaID,
        elector: RoundRobinLeaderElector,
        machine: Machine<S>,
        net_adaptor: NetworkAdaptor,
    ) -> Pacemaker<S>
    where
        Self: Sized,
    {
        let (notifier, timeout_ch) = tokio::sync::mpsc::channel(1);
        let max_view_timeout = 60_000;
        let rtt = 10_000;
        let timer = DefaultTimer::new(notifier, max_view_timeout, rtt);

        Self {
            view: 0,
            id,
            elector,
            sync_state: Arc::new(AtomicU8::new(SYNC_IDLE)),
            pending_prop: BinaryHeap::with_capacity(8),
            net_adaptor,
            timer,
            timeout_ch,
            machine,
            ready_queue: VecDeque::with_capacity(8),
        }
    }

    /// Start processing events. Stop once received signal from `quit_ch`.
    pub async fn run(&mut self, mut quit_ch: TchanR<()>) -> io::Result<()> {
        info!("pacemaker up");
        let mut quit = false;

        let mut interval = tokio::time::interval(Duration::from_millis(500));
        interval.tick().await;

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // refactor
                    LivenessStorage::flush(self.liveness_storage()).await.unwrap();
                },
                Some(()) = quit_ch.recv() => {
                    quit = true;
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

        info!("flushing...");
        LivenessStorage::flush(self.liveness_storage())
            .await
            .unwrap();

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
                        let leaf_height =
                            LivenessStorage::get_leaf(self.liveness_storage()).height();
                        self.check_pending_prop(leaf_height).await?
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
                if self.liveness_storage().is_qc_node_exists(prop.justify()) {
                    self.emit_safety_event(SafetyEvent::RecvProposal(ctx, Arc::new(*prop)));
                } else {
                    self.pending_prop.push(SafetyEventWrapper::from(
                        prop.height(),
                        SafetyEvent::RecvProposal(ctx, Arc::new(*prop)),
                    ));
                    let height = LivenessStorage::get_leaf(self.liveness_storage()).height();
                    self.check_pending_prop(height).await?;
                }
            }
            PeerEvent::AcceptProposal { ctx, prop, sign } => {
                if let Some(sign) = sign {
                    self.emit_safety_event(SafetyEvent::RecvSign(ctx, Arc::new(*prop), sign));
                }
            }
            PeerEvent::Timeout { ctx, tc } => {
                info!("recv timeout msg view={} from {}", ctx.view, &ctx.from);
                match self.liveness_storage().append_tc(tc) {
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
                match self.liveness_storage().fetch_branch(&strategy) {
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
                if let Err(e) =
                    LivenessStorage::update_qc_high(self.liveness_storage(), &node, node.justify())
                {
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
            let next_prop_view = -sw.view as ViewNumber;
            if next_prop_view <= leaf_height + 1 {
                self.emit_safety_event(sw.se);
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
                    let grow_from = self.liveness_storage().get_locked_node().height();

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
    fn emit_safety_event(&mut self, se: SafetyEvent) {
        // self.machine_adaptor.safety().send(se).await.unwrap();
        match self.machine.process_safety_event(se) {
            Ok(ready) => {
                if let Ready::Nil = ready {
                    return;
                }
                self.ready_queue.push_back(ready);
            }
            Err(e) => {
                error!("{:?}", e);
            }
        }
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

    #[inline]
    fn liveness_storage(&mut self) -> &mut S {
        self.machine.storage()
    }
}

#[test]
#[ignore = "tested"]
fn tokio_tick() {
    use std::time::Duration;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        interval.tick().await;
        let start = tokio::time::Instant::now();
        println!("time:{:?}", start);
        loop {
            interval.tick().await;
            println!("time:{:?}", start.elapsed());
        }
    });
}
