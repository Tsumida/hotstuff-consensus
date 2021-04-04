//! Hotstuff storage.

// pub mod sqlite;

pub mod persistor;
pub mod recover;

use persistor::{FlushTask, MySQLStorage};
use recover::*;

use cryptokit::DefaultSignaturer;
use hotstuff_rs::safety::machine::{SafetyStorage, Snapshot};
use hs_data::*;
use log::{debug, error, info};
use pacemaker::{
    data::{combine_time_certificates, BranchData, BranchSyncStrategy, TimeoutCertificate},
    liveness_storage::{LivenessStorage, LivenessStorageErr},
};
use sqlx::mysql::MySqlRow;
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::Arc,
    time::SystemTime,
};

use thiserror::Error;
#[derive(Debug, Clone, Error)]
pub enum HotStuffStroageErr {
    #[error("falied to flush, internal err")]
    FailedToFlush,
}

#[derive(Clone)]
pub struct InMemoryState {
    // Token of the hotstuff system.
    token: String,

    // Leaf is the node with justify=qc-high.
    leaf: Arc<TreeNode>,

    // Qc-high is the QC with higest view this replica has ever seen.
    qc_high: Arc<GenericQC>,

    // height of last voted proposal this replica has ever seen.
    vheight: ViewNumber,

    current_view: ViewNumber,

    // height for last committed
    committed_height: ViewNumber,

    // Lasted executed proposal.
    last_commit: Arc<TreeNode>,

    // Last locked proposal.
    b_locked: Arc<TreeNode>,
}

/// Create a HotStuffStorage using in memory storage(Mysql disabled).
pub fn init_in_mem_storage(
    total: usize,
    init_node: &TreeNode,
    init_node_hash: &NodeHash,
    init_qc: &GenericQC,
    self_id: ReplicaID,
    peers_addr: HashMap<ReplicaID, String>,
    signaturer: DefaultSignaturer,
) -> HotstuffStorage {
    let token = format!("hotstuff_test");

    let conf = HotStuffConfig {
        token: token.clone(),
        total,
        replica_id: self_id,
        peers_addr,
    };

    let mut hss = HotstuffStorage::new(token, init_node, init_qc, conf, signaturer, None);
    hss.in_mem_queue
        .insert(init_node_hash.clone(), Arc::new(init_node.clone()));
    hss
}

/// Create a HotStuffStorage with MySQL enabled.
pub async fn init_hotstuff_storage(
    token: String,
    total: usize,
    init_node: &TreeNode,
    init_node_hash: &NodeHash,
    self_id: ReplicaID,
    peers_addr: HashMap<ReplicaID, String>,
    mysql_addr: &str,
    signaturer: DefaultSignaturer,
) -> HotstuffStorage {
    let conn_pool = sqlx::pool::PoolOptions::new()
        .max_connections(2)
        .connect(mysql_addr)
        .await
        .unwrap();

    let backend = Some(MySQLStorage::new(conn_pool));

    let conf = HotStuffConfig {
        token: token.clone(),
        total,
        replica_id: self_id,
        peers_addr,
    };

    let mut hss = HotstuffStorage::new(
        token,
        &hs_data::INIT_NODE,
        &hs_data::INIT_QC,
        conf,
        signaturer,
        backend,
    );
    hss.init().await;
    hss.in_mem_queue
        .insert(init_node_hash.clone(), Arc::new(init_node.clone()));
    hss
}

/// HotstuffStorage is based on mysql and promises that:
/// - It won't lose any proposal with a QuorumCertificate, once received.
/// - It won't lose any QuorumCertificate and TimeCertificate, once received.
///
/// For example , Persistent Stroage keeps `3<-4` in branch `3<-4<-4` and never lose them.
/// QC carried by the second `4` should be stablized but no guarantees for the second proposal `4` itself.
pub struct HotstuffStorage {
    // None for disable MySQL backend.
    backend: Option<MySQLStorage>,
    state: InMemoryState,
    conf: HotStuffConfig,
    signaturer: DefaultSignaturer,

    in_mem_queue: HashMap<NodeHash, Arc<TreeNode>>,

    tc_map: BTreeMap<ViewNumber, HashSet<TimeoutCertificate>>,
    new_view_set: BTreeMap<ViewNumber, HashMap<ReplicaID, Arc<GenericQC>>>,

    // dirty data.
    combined_tc_queue: VecDeque<TimeoutCertificate>,
    partial_tc_queue: VecDeque<TimeoutCertificate>,
    prop_queue: VecDeque<Arc<TreeNode>>,
    justify_queue: VecDeque<GenericQC>,

    dirty: bool,
}

pub struct HotStuffConfig {
    // Hotstuff system token.
    pub token: String,
    // Peers number.
    pub total: usize,
    pub replica_id: ReplicaID,
    pub peers_addr: HashMap<ReplicaID, String>,
}

impl HotStuffConfig {
    pub fn threshold(&self) -> usize {
        (self.total << 1) / 3
    }
}

impl HotstuffStorage {
    pub fn new(
        token: String,
        init_node: &TreeNode,
        init_qc: &GenericQC,
        conf: HotStuffConfig,
        signaturer: DefaultSignaturer,
        backend: Option<MySQLStorage>,
    ) -> Self
    where
        Self: Sized,
    {
        let storage = Self {
            state: InMemoryState {
                token,
                current_view: 0,
                vheight: 0,
                committed_height: 0,
                last_commit: Arc::new(init_node.clone()),
                b_locked: Arc::new(init_node.clone()),
                // safety related
                leaf: Arc::new(init_node.clone()),
                qc_high: Arc::new(init_qc.clone()),
            },
            backend,
            signaturer,
            conf,
            tc_map: BTreeMap::new(),
            in_mem_queue: HashMap::new(),
            combined_tc_queue: VecDeque::with_capacity(8),
            partial_tc_queue: VecDeque::with_capacity(8),
            prop_queue: VecDeque::with_capacity(8),
            justify_queue: VecDeque::with_capacity(8),
            dirty: false,
            new_view_set: BTreeMap::new(),
        };
        // storage.append_new_node(init_node);
        storage
    }

    pub async fn init(&mut self) {
        if let Some(backend) = self.backend.as_mut() {
            backend.init(&self.conf, &self.signaturer).await;
        }
    }

    pub async fn recover(token: String, addr: &str) -> HotstuffStorage {
        // refactor
        // load all proposals;

        let conn_pool = sqlx::MySqlPool::connect(addr).await.unwrap();

        info!("start recovering");

        let st = SystemTime::now();
        let conf = recover_hotstuff_config(&conn_pool).await;
        let mut dur_recover_state = SystemTime::now().duration_since(st).unwrap();
        info!(
            "recover hotstuff configuration, took {} ms",
            dur_recover_state.as_millis()
        );

        let st_1 = SystemTime::now();
        let in_mem_queue = sqlx::query(
            "
            select * from proposal inner join qc 
            on proposal.justify_view=qc.view 
            order by proposal.justify_view;
            ",
        )
        .map(|row: MySqlRow| map_row_to_proposal(row))
        .fetch_all(&conn_pool)
        .await
        .unwrap()
        .into_iter()
        .collect::<HashMap<NodeHash, Arc<TreeNode>>>();

        dur_recover_state = SystemTime::now().duration_since(st_1).unwrap();
        info!(
            "recover in-mem proposal tree, took {} ms",
            dur_recover_state.as_millis()
        );

        let st_2 = SystemTime::now();
        let state = recover_hotstuff_state(&conn_pool).await;
        dur_recover_state = SystemTime::now().duration_since(st_2).unwrap();
        info!(
            "recover in mem hotstuff state, took {} ms",
            dur_recover_state.as_millis()
        );

        let st_3 = SystemTime::now();
        let tc_map = recover_tc_map(&conn_pool).await;
        dur_recover_state = SystemTime::now().duration_since(st_3).unwrap();
        info!(
            "recover tc mapper, took {} ms",
            dur_recover_state.as_millis()
        );

        // recover crypto
        let st_4 = SystemTime::now();
        let signaturer = recover_signaturer(&token, &conn_pool).await;
        dur_recover_state = SystemTime::now().duration_since(st_4).unwrap();
        info!(
            "recover signaturer, took {} ms",
            dur_recover_state.as_millis()
        );

        // update in memory.
        let end = SystemTime::now();
        let recover_dur = end.duration_since(st).unwrap();

        info!(
            "recover done, time consumed: {:.4} s",
            recover_dur.as_secs_f32(),
        );

        HotstuffStorage {
            backend: Some(MySQLStorage::new(conn_pool)),
            state,
            conf,
            signaturer,
            tc_map,
            in_mem_queue,
            combined_tc_queue: VecDeque::with_capacity(8),
            partial_tc_queue: VecDeque::with_capacity(8),
            prop_queue: VecDeque::with_capacity(8),
            justify_queue: VecDeque::with_capacity(8),
            dirty: false,
            new_view_set: BTreeMap::new(),
        }
    }

    // refactor
    pub async fn async_flush(&mut self) -> Result<(), sqlx::Error> {
        if !self.dirty {
            return Ok(());
        }

        if let Some(backend) = self.backend.as_mut() {
            let mut prop_queue = VecDeque::with_capacity(self.prop_queue.len());
            prop_queue.append(&mut self.prop_queue);

            let mut justify_queue = VecDeque::with_capacity(self.justify_queue.len());
            justify_queue.append(&mut self.justify_queue);

            let mut partial_tc_queue = VecDeque::with_capacity(self.partial_tc_queue.len());
            partial_tc_queue.append(&mut self.partial_tc_queue);

            let mut combined_tc_queue = VecDeque::with_capacity(self.combined_tc_queue.len());
            combined_tc_queue.append(&mut self.combined_tc_queue);

            let state = self.state.clone();

            let task = FlushTask::asynch(
                combined_tc_queue,
                partial_tc_queue,
                prop_queue,
                justify_queue,
                state,
            );

            backend.async_flush(task).await;

            self.reset_dirty();
        }

        Ok(())
    }

    #[inline]
    pub fn threshold(&self) -> usize {
        (self.conf.total << 1) / 3
    }

    fn compress_tc(&mut self) -> Option<ViewNumber> {
        // compress
        let view = self
            .tc_map
            .iter()
            .filter(|(_, s)| s.len() >= self.conf.threshold())
            .max_by_key(|(k, _)| *k)
            .map(|(view, _)| *view);

        if let Some(v) = view {
            let from = String::default();
            self.tc_map
                .remove(&v)
                .and_then(|tc| {
                    combine_time_certificates(
                        &self.signaturer,
                        &from,
                        v,
                        self.get_qc_high().as_ref().clone(),
                        &tc,
                    )
                })
                .map_or((), |combined_tc| {
                    self.combined_tc_queue.push_back(combined_tc);
                });
        }
        view
    }

    fn get(&self, node_hash: &NodeHash) -> Option<Arc<TreeNode>> {
        // refactor
        match self.in_mem_queue.get(node_hash) {
            None => None,
            Some(s) => Some(s.clone()),
        }
    }

    fn insert(&mut self, node_hash: NodeHash, prop: Arc<TreeNode>) {
        self.in_mem_queue.insert(node_hash.clone(), prop.clone());
        self.prop_queue.push_back(prop);
    }

    #[inline]
    fn set_dirty(&mut self) {
        self.dirty = true;
    }

    #[inline]
    fn reset_dirty(&mut self) {
        self.dirty = false;
    }
}

#[async_trait::async_trait]
impl SafetyStorage for HotstuffStorage {
    async fn flush(&mut self) -> hotstuff_rs::safety::machine::Result<()> {
        self.async_flush()
            .await
            .map_err(|e| hotstuff_rs::safety::machine::SafetyErr::InternalErr(Box::new(e)))
    }

    fn append_new_node(&mut self, node: &TreeNode) {
        let h = TreeNode::hash(node);
        let node = Arc::new(node.clone());

        // Warning: OOM since every node has a replica in memory.
        self.insert(h, node);
        self.set_dirty();
    }

    fn append_new_qc(&mut self, qc: &GenericQC) {
        self.justify_queue.push_back(qc.clone());
        self.set_dirty();
    }

    fn find_three_chain(&self, node: &TreeNode) -> Vec<Arc<TreeNode>> {
        let mut chain = Vec::with_capacity(3);
        if let Some(b3) = self.get(node.justify().node_hash()) {
            chain.push(b3.clone());
            if let Some(b2) = self.get(b3.justify().node_hash()) {
                chain.push(b2.clone());
                if let Some(b1) = self.get(b2.justify().node_hash()) {
                    chain.push(b1.clone());
                }
            }
        }
        chain
    }

    /// Persistent
    fn update_leaf(&mut self, new_leaf: &TreeNode) {
        let hash = TreeNode::hash(new_leaf);
        self.state.vheight = new_leaf.height();
        self.state.leaf = Arc::new(new_leaf.clone());
        self.insert(hash, self.state.leaf.clone());
        self.set_dirty();
    }

    fn get_leaf(&self) -> Arc<TreeNode> {
        self.state.leaf.clone()
    }

    fn get_qc_high(&self) -> Arc<GenericQC> {
        self.state.qc_high.clone()
    }

    /// qc_high.node == qc_node.
    fn update_qc_high(&mut self, new_qc_node: &TreeNode, new_qc_high: &GenericQC) {
        if let Some(qc_node) = self.get(self.get_qc_high().node_hash()) {
            if new_qc_node.height() > qc_node.height() {
                self.state.qc_high = Arc::new(new_qc_high.clone());
                self.update_leaf(new_qc_node);
                self.append_new_qc(new_qc_high);
                debug!("update qc-high(h={})", new_qc_node.height());
                self.set_dirty();
            }
        }
    }

    fn is_conflicting(&self, a: &TreeNode, b: &TreeNode) -> bool {
        let (a, b) = if a.height() >= b.height() {
            (a, b)
        } else {
            (b, a)
        };

        // a.height() >= b.height()
        let mut height_now = a.height();
        let mut parent_hash = a.parent_hash().clone();

        // refactor
        let mut node: Arc<TreeNode> = Arc::new(a.clone());
        while height_now > b.height() {
            if let Some(prev) = self.get(&parent_hash) {
                node = prev.clone();
                height_now = prev.height();
                // if prev == init_qc, then prev.parent() points to an invalied node.
                parent_hash = prev.parent_hash().clone();
            } else {
                break;
            }
        }

        TreeNode::hash(node.as_ref()) != TreeNode::hash(b)
    }

    fn get_node(&self, node_hash: &NodeHash) -> Option<Arc<TreeNode>> {
        self.get(node_hash).and_then(|node| Some(node.clone()))
    }

    fn get_locked_node(&self) -> Arc<TreeNode> {
        self.state.b_locked.clone()
    }

    // Persistent
    fn update_locked_node(&mut self, node: &TreeNode) {
        debug!("locked at node with height {}", node.height());
        self.state.b_locked = Arc::new(node.clone());
        self.set_dirty();
    }

    fn get_last_committed(&self) -> Arc<TreeNode> {
        self.state.last_commit.clone()
    }

    fn get_view(&self) -> ViewNumber {
        self.state.current_view
    }

    // Persistent
    fn increase_view(&mut self, new_view: ViewNumber) {
        self.state.current_view = ViewNumber::max(self.state.current_view, new_view);
        self.set_dirty();
    }

    fn is_consecutive_three_chain(&self, chain: &Vec<impl AsRef<TreeNode>>) -> bool {
        if chain.len() != 3 {
            debug!("not consecutive 3-chain, len={}", chain.len());
            return false;
        }

        let b_3 = chain.get(0).unwrap().as_ref();
        let b_2 = chain.get(1).unwrap().as_ref();
        let b = chain.get(2).unwrap().as_ref();

        let pred_32 = b_3.parent_hash() == &TreeNode::hash(b_2);
        let pred_21 = b_2.parent_hash() == &TreeNode::hash(b);
        // &b_3.parent_hash() == &TreeNode::hash(b_2) && &b_2.parent_hash() == &TreeNode::hash(b)
        debug!(
            "consecutive judge with h = {},{},{}: {} - {}",
            b_3.height(),
            b_2.height(),
            b.height(),
            pred_32,
            pred_21
        );
        pred_32 && pred_21
    }

    fn get_vheight(&self) -> ViewNumber {
        self.state.vheight
    }

    // Persistent
    fn update_vheight(&mut self, vheight: ViewNumber) -> ViewNumber {
        let prev = self.state.vheight;
        self.state.vheight = ViewNumber::max(self.state.vheight, vheight);
        self.set_dirty();

        prev
    }

    // TODO: add informer for watchers.
    fn commit(&mut self, to_commit: &TreeNode) {
        if self.state.committed_height >= to_commit.height() {
            debug!("to_commit with smaller height {}", to_commit.height());
            return;
        }
        let to_committed_height = to_commit.height();
        for h in self.state.committed_height + 1..=to_committed_height {
            // TODO:execute,
            self.state.committed_height = h;
        }
        self.state.last_commit = Arc::new(to_commit.clone());
        self.set_dirty();

        info!(
            "commit new proposal, committed_height = {}",
            self.state.committed_height
        );
    }

    fn hotstuff_status(&self) -> Snapshot {
        Snapshot {
            view: self.state.current_view,
            leader: None,
            qc_high: Box::new(self.state.qc_high.as_ref().clone()),
            leaf: Box::new(self.state.leaf.as_ref().clone()),
            locked_node: Box::new(self.state.b_locked.as_ref().clone()),
            last_committed: self.state.committed_height,
        }
    }
}

#[async_trait::async_trait]
impl LivenessStorage for HotstuffStorage {
    async fn flush(&mut self) -> Result<(), LivenessStorageErr> {
        self.async_flush()
            .await
            .map_err(|_| LivenessStorageErr::FailedToFlush)
    }

    fn append_tc(&mut self, tc: TimeoutCertificate) -> Result<ViewNumber, LivenessStorageErr> {
        match tc.view_sign() {
            pacemaker::data::SignType::Partial(_) => {
                self.partial_tc_queue.push_back(tc);
            }
            pacemaker::data::SignType::Combined(_) => {
                self.combined_tc_queue.push_back(tc);
            }
        }
        self.set_dirty();

        // current_view - 1 is ok
        Ok(self
            .compress_tc()
            .unwrap_or(self.get_view().saturating_sub(1)))
    }

    fn is_reach_threshold(&self, view: ViewNumber) -> bool {
        self.tc_map
            .get(&view)
            .map_or(false, |s| s.len() >= self.conf.threshold())
    }

    fn update_qc_high(
        &mut self,
        node: &TreeNode,
        qc: &GenericQC,
    ) -> Result<(), LivenessStorageErr> {
        <HotstuffStorage as SafetyStorage>::update_qc_high(self, node, qc);
        self.set_dirty();

        Ok(())
    }

    fn fetch_branch(
        &self,
        strategy: &BranchSyncStrategy,
    ) -> Result<BranchData, LivenessStorageErr> {
        // refactor: batch_size
        // Find proposals in memory through
        match strategy {
            BranchSyncStrategy::Grow { grow_from, .. } => {
                let mut v: Vec<TreeNode> = self
                    .in_mem_queue
                    .values()
                    .filter(|s| &s.height() > grow_from)
                    .map(|node| node.as_ref().clone())
                    .collect();

                v.sort_by(|a, b| a.height().cmp(&b.height()));

                Ok(BranchData { data: v })
            }
        }
    }

    fn is_qc_node_exists(&mut self, qc: &GenericQC) -> bool {
        self.get(&qc.node_hash()).is_some()
    }

    fn get_locked_node(&mut self) -> &TreeNode {
        self.state.b_locked.as_ref()
    }

    fn get_leaf(&self) -> &TreeNode {
        self.state.leaf.as_ref()
    }

    fn increase_view(&mut self, new_view: ViewNumber) {
        <Self as SafetyStorage>::increase_view(self, new_view);
    }

    fn new_view_set(&mut self, view: ViewNumber) -> &mut HashMap<ReplicaID, Arc<GenericQC>> {
        self.new_view_set.entry(view).or_insert(HashMap::new())
    }

    #[inline]
    fn get_threshold(&mut self) -> usize {
        self.threshold()
    }

    fn clean_new_view_set(&mut self, view_threshold: ViewNumber) {
        // BTreeMap::retain is unstable.
        let new_tree = self.new_view_set.split_off(&view_threshold);
        self.new_view_set = new_tree;
    }
}
