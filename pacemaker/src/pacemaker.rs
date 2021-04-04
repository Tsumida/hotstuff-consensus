use crate::{
    data::{BranchSyncStrategy, PeerEvent, SyncStatus, TimeoutCertificate},
    elector::RoundRobinLeaderElector,
    liveness_storage::LivenessStorage,
    network::*,
    timer::{DefaultTimer, TimeoutEvent},
};
use cryptokit::DefaultSignaturer;
use hotstuff_rs::safety::machine::{
    self, Machine, Ready, Safety, SafetyEvent, SafetyStorage, Snapshot,
};
use hs_data::{msg::Context, GenericQC, ReplicaID, SignKit, TreeNode, Txn, ViewNumber};
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
use once_cell::sync::OnceCell;

pub type TchanR<T> = tokio::sync::mpsc::Receiver<T>;
pub type TchanS<T> = tokio::sync::mpsc::Sender<T>;
pub type PeerEventRecvr = TchanR<PeerEvent>;
pub type PeerEventSender = TchanS<PeerEvent>;
pub type CtlRecvr = tokio::sync::broadcast::Receiver<()>;
pub type CtlSender = tokio::sync::broadcast::Sender<()>;

const SYNC_SYNCHRONIZING: u8 = 0;
const SYNC_IDLE: u8 = 1;

/// Flushing interval.
pub static FLUSH_INTERVAL: OnceCell<u64> = OnceCell::new();

/// Non-leader nodes wait for a while for synchronizing.
pub static DUR_REPLICA_WAIT: OnceCell<u64> = OnceCell::new();

pub static BATCH_SIZE: usize = 8;

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
#[ignore = "tested"]
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
    elector: crate::elector::RoundRobinLeaderElector,

    signaturer: DefaultSignaturer,

    machine: Machine<S>,
    ready_queue: VecDeque<Ready>,
    pending_prop: BinaryHeap<SafetyEventWrapper>,

    sync_state: Arc<AtomicU8>,

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
        signaturer: DefaultSignaturer,
    ) -> Pacemaker<S>
    where
        Self: Sized,
    {
        let (notifier, timeout_ch) = tokio::sync::mpsc::channel(1);

        // refactor
        let max_view_timeout = 30_000;
        let rtt = 12_000;
        let timer = DefaultTimer::new(notifier, max_view_timeout, rtt);

        Self {
            view: 0,
            id,
            elector,
            signaturer,
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

        self.goto_new_view(1).await?;

        let mut interval = tokio::time::interval(Duration::from_millis(
            *FLUSH_INTERVAL.get().expect("var uninit"),
        ));
        interval.tick().await;

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // refactor
                    LivenessStorage::flush(self.liveness_storage()).await.unwrap();

                    while let Some(ready) = self.ready_queue.pop_front(){
                        self.process_ready_event(ready).await.unwrap();
                    }
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

        // refactor
        info!("flushing...");
        LivenessStorage::flush(self.liveness_storage())
            .await
            .unwrap();

        info!("pacemaker down");
        Ok(())
    }

    async fn process_timeout_event(&mut self, te: TimeoutEvent) -> io::Result<()> {
        match te {
            TimeoutEvent::ViewTimeout(view) => self.process_local_timeout(view).await,
        }
    }

    /// Try to go to view=`new_view`+1.
    async fn process_local_timeout(&mut self, timeout_view: ViewNumber) -> io::Result<()> {
        if timeout_view >= self.view {
            // generate tc and save it to local storage.
            let qc_high = self.liveness_storage().get_qc_high().as_ref().clone();
            let tc = self.sign_tc(self.view, qc_high);

            // ignore error
            if let Err(e) = LivenessStorage::append_tc(self.liveness_storage(), tc.clone()) {
                error!("failed to save tc to local storage: {:?}", e);
            }

            self.emit_peer_event(PeerEvent::Timeout {
                ctx: Context::broadcast(self.id.clone(), self.view),
                tc: tc,
            })
            .await
            .unwrap();

            // self.liveness_storage().append_tc(tc).unwrap();

            // goto new view and reset timer.
            self.goto_new_view(timeout_view + 1).await?;

            let current_view = self.view;
            LivenessStorage::increase_view(self.liveness_storage(), current_view);

            // Reuse local timeout event for branch synchronizing timeout.
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
                let leaf_height = LivenessStorage::get_leaf(self.liveness_storage()).height();
                return self.check_pending_prop(leaf_height).await;
            }
        }

        Ok(())
    }

    // todo
    pub async fn process_network_event(&mut self, net_event: PeerEvent) -> io::Result<()> {
        match net_event {
            PeerEvent::NewProposal { ctx, prop } => {
                /*
                if self.liveness_storage().is_qc_node_exists(prop.justify()) {
                    // refactor: remove this branch
                    self.emit_safety_event(SafetyEvent::RecvProposal(ctx, Arc::new(*prop)));
                } else {
                    */

                self.pending_prop.push(SafetyEventWrapper::from(
                    // use prop.justify.view, not prop.height
                    prop.justify().view(),
                    SafetyEvent::RecvProposal(ctx, Arc::new(*prop)),
                ));
                let height = LivenessStorage::get_leaf(self.liveness_storage()).height();
                self.check_pending_prop(height).await?;
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
                            self.process_local_timeout(v).await?;
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
                            ctx: Context::single(self.id.clone(), ctx.from, self.view),
                            strategy,
                            branch: Some(bd),
                            status: SyncStatus::Success,
                        })
                        .await?;
                    }
                    Err(e) => {
                        error!("{:?}", e);
                        self.emit_peer_event(PeerEvent::BranchSyncResponse {
                            ctx: Context::single(self.id.clone(), ctx.from, self.view),
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
                SyncStatus::Success => {
                    if let Some(branch) = branch {
                        self.emit_safety_event(SafetyEvent::BranchSync(ctx, branch.data));
                    }
                }
                other => error!("branch sync error: {:?}", other),
            },
            PeerEvent::Ping { .. } => {}
            PeerEvent::NewView { ctx, qc_high } => {
                // collect at least n-f new-view
                info!(
                    "recv new-view msg from {} with justify.height = {}, view = {}",
                    &ctx.from,
                    qc_high.view(),
                    ctx.view,
                );
                let qc = Arc::new(*qc_high);
                self.liveness_storage()
                    .new_view_set(ctx.view)
                    .insert(ctx.from.clone(), qc.clone());
                self.emit_safety_event(SafetyEvent::RecvNewViewMsg(ctx, qc));
            }
        }
        Ok(())
    }

    pub async fn process_ready_event(&mut self, ready: machine::Ready) -> io::Result<()> {
        match ready {
            machine::Ready::Nil => {}
            machine::Ready::InternalState(_, _) => {}
            machine::Ready::NewProposal(ctx, prop) => {
                // Brocast new proposal to all replicas.
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
                let flag = match ctx.to {
                    hs_data::msg::DeliveryType::Single(ref to) => to != &self.id,
                    _ => unreachable!(),
                };

                self.emit_peer_event(PeerEvent::AcceptProposal {
                    ctx,
                    prop: Box::new(prop.as_ref().clone()),
                    sign: Some(sign),
                })
                .await?;

                // responsiveness, goto next view right now.
                if flag {
                    // replicas wait for brief time so that leader goes to next view quicker.
                    // refactor
                    tokio::time::sleep(Duration::from_millis(
                        *DUR_REPLICA_WAIT.get().expect("var uninit"),
                    ))
                    .await;
                    self.process_local_timeout(prop.height()).await?;
                }
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
            Ready::ProposalReachConsensus(prop_view) => {
                // responsiveness, goto next view right now.
                self.process_local_timeout(prop_view).await?;
            }
        }
        Ok(())
    }

    /// Check pending queue, start branch synchronization if there are proposals lack of `justify.node` .
    pub async fn check_pending_prop(&mut self, leaf_height: ViewNumber) -> io::Result<()> {
        if let Some(sw) = self.pending_prop.pop() {
            let justify_view = -sw.view as ViewNumber;
            if justify_view <= leaf_height {
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
                    // let grow_from = self.liveness_storage().get_locked_node().height();
                    let grow_from = leaf_height;
                    info!(
                        "start branch sync: from={}, batch-size={}]",
                        grow_from, BATCH_SIZE
                    );

                    // note:
                    let ctx = Context::single(
                        self.id.clone(),
                        self.elector.get_leader(self.view).clone(),
                        self.view,
                    );
                    self.emit_peer_event(PeerEvent::BranchSyncRequest {
                        ctx,
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

    // todo: consider use Pacemaker Error
    pub async fn emit_peer_event(&mut self, pe: PeerEvent) -> io::Result<()> {
        let dur = self.timer.timeout_by_delay();
        if let Err(_) = self.net_adaptor.event_sender.send_timeout(pe, dur).await {
            error!("pacemaker -> network adaptor block and timeout");
        }

        Ok(())
    }

    #[inline]
    /// Communicate with Machine.
    pub fn emit_safety_event(&mut self, se: SafetyEvent) {
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

    fn update_pm_status(&mut self, view: ViewNumber) {
        self.view = ViewNumber::max(self.view, view);
        let new_view = self.view;
        LivenessStorage::increase_view(self.liveness_storage(), new_view);
        info!(
            "view = {} with leader = {:?}",
            self.view,
            self.elector.get_leader(self.view)
        );
    }

    /// Go to new view and reset state. This function will increase view.
    async fn goto_new_view(&mut self, new_view: ViewNumber) -> io::Result<()> {
        self.timer
            .view_timeout(new_view, self.timer.timeout_by_delay());

        self.update_pm_status(new_view);

        let new_view = self.view;
        self.liveness_storage().clean_new_view_set(new_view);

        let qc_high = self.liveness_storage().get_qc_high();
        let ctx = Context::single(
            self.id.clone(),
            self.elector.get_leader(self.view).to_string(),
            self.view,
        );
        self.emit_peer_event(PeerEvent::NewView {
            ctx,
            qc_high: Box::new(qc_high.as_ref().clone()),
        })
        .await
    }

    #[inline]
    fn liveness_storage(&mut self) -> &mut S {
        self.machine.storage()
    }

    // refactor:
    fn sign_tc(&self, view: ViewNumber, qc_high: GenericQC) -> TimeoutCertificate {
        let sign_kit = SignKit::from((
            self.signaturer.sks.sign(&view.to_be_bytes()),
            self.signaturer.sign_id,
        ));

        TimeoutCertificate::new(self.id.clone(), self.view, sign_kit, qc_high)
    }

    /// Expose safety module.
    pub fn safety_module(&mut self) -> &mut dyn Safety {
        &mut self.machine
    }

    /// Return Some(Ready) if and only if there are at least n-f NewView.
    pub fn make_new_proposal(&mut self, cmds: Vec<Txn>) -> Option<machine::Ready> {
        let th = self.liveness_storage().get_threshold();
        let current_view = self.view;
        let lens = self.liveness_storage().new_view_set(current_view).len();

        // refactor: <= to <
        if lens <= th {
            error!(
                "new-view msg not enough, need at least {}, got {}",
                th + 1,
                lens
            );
            return None;
        } else {
            // there are no way to form another set with at least n-f new-view msgs.
            let current_view = self.view + 1;
            self.liveness_storage().clean_new_view_set(current_view);
            Some(self.safety_module().on_beat(cmds).unwrap())
        }
    }
}

use tokio::sync::oneshot;

#[derive(Debug, Clone)]
pub struct PacemakerState {
    pub replica_id: ReplicaID,
    pub current_view: ViewNumber,
    pub current_leader: Option<ReplicaID>,
    pub machine_snapshot: Snapshot,
}

/// Event loop for pacemaker with observer, informer and other hooks.
pub async fn event_loop_with_functions<S>(
    mut pm: Pacemaker<S>,
    mut quit_ch: TchanR<()>,
    observe_fn: impl Fn() -> Option<oneshot::Sender<PacemakerState>>,
    _: impl Fn(Arc<TreeNode>),
    fetch: impl Fn(bool) -> Option<Vec<Txn>>,
    end_fn: impl Fn(ViewNumber) -> bool,
    finilizer: impl FnOnce(),
) -> std::io::Result<()>
where
    S: SafetyStorage + LivenessStorage + Send + Sync,
{
    info!("pacemaker up");
    let mut quit = false;

    pm.goto_new_view(1).await?;

    let mut interval = tokio::time::interval(Duration::from_millis(
        *FLUSH_INTERVAL.get().expect("var uninit"),
    ));
    interval.tick().await;

    while !quit {
        // process new-tx
        let current_view = pm.view;
        if end_fn(current_view) {
            break;
        }
        let enable_make_prop = pm.liveness_storage().new_view_set(current_view).len()
            > pm.liveness_storage().get_threshold();

        if let Some(cmds) = fetch(enable_make_prop) {
            let ready = pm.make_new_proposal(cmds);
            match ready {
                Some(ready) => pm.ready_queue.push_back(ready),
                None => {
                    error!("Can't form new proposal: waiting for at least n-f new-view messaegs")
                }
            }
        }

        // todo: process querying about hotstuff node.
        if let Some(sender) = observe_fn() {
            let pm_state = PacemakerState {
                current_view: pm.view,
                current_leader: Some(pm.elector.get_leader(pm.view).clone()),
                replica_id: pm.id.clone(),
                machine_snapshot: pm.safety_module().take_snapshot(),
            };
            let _ = sender.send(pm_state);
        }

        tokio::select! {
            // pm event.
            _ = interval.tick() => {
                // refactor
                LivenessStorage::flush(pm.liveness_storage()).await.unwrap();
            },
            Some(()) = quit_ch.recv() => {
                quit = true;
            },
            Some(net_event) = pm.net_adaptor.event_recvr.recv() => {
                pm.process_network_event(net_event).await?;
            },
            Some(te) = pm.timeout_ch.recv() => {
                pm.process_timeout_event(te).await?;
            },
        }

        while let Some(ready) = pm.ready_queue.pop_front() {
            pm.process_ready_event(ready).await.unwrap();
        }
    }

    // refactor
    info!("flushing...");
    LivenessStorage::flush(pm.liveness_storage()).await.unwrap();
    info!("pacemaker down");

    finilizer();
    Ok(())
}
