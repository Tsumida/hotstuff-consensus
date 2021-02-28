//! Hotstuff storage.

// pub mod sqlite;

use hotstuff_rs::safety::machine::{SafetyStorage, Snapshot};
use hs_data::*;
use hs_data::{TreeNode, ViewNumber};
use log::{debug, info};
use pacemaker::{
    data::{BranchData, BranchSyncStrategy, TimeoutCertificate},
    liveness_storage::{LivenessStorage, LivenessStorageErr},
};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

pub struct InMemoryState {
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
    b_executed: Arc<TreeNode>,

    // Last locked proposal.
    b_locked: Arc<TreeNode>,
}

/// HotstuffStorage cache changes in memory and flush dirty data.
pub struct HotstuffStorage {
    backend: PersistentStorage,
    state: InMemoryState,

    tc_queue: VecDeque<TimeoutCertificate>,
    /// consider case: `3<-4<-4`
    /// proposal `3<-4` should be appended into prop_queue and
    /// the second 4's qc should be appended into justify_queue.
    prop_queue: VecDeque<Arc<TreeNode>>,
    justify_queue: VecDeque<GenericQC>,
}

/// Persistent storage based on mysql. PersistentStorage promises that:
/// - It won't lose any proposal with a QuorumCertificate, once received.
/// - It won't lose any QuorumCertificate and TimeCertificate, once received.
///
/// For example , Persistent Stroage keeps `3<-4` in branch `3<-4<-4` and never lose them.
/// QC carried by the second `4` should be stablized but no guarantees for the second proposal `4` itself.
///
pub struct PersistentStorage {
    // A pool for proposals that doesn't form qc.
    in_mem_queue: HashMap<NodeHash, Arc<TreeNode>>,
    conn_pool: sqlx::MySqlPool,
}

impl PersistentStorage {
    fn get(&self, node_hash: &NodeHash) -> Option<Arc<TreeNode>> {
        // search cache.
        match self.in_mem_queue.get(node_hash) {
            // search table .
            None => None,
            Some(s) => Some(s.clone()),
        }
    }

    fn insert(&mut self, node_hash: NodeHash, prop: Arc<TreeNode>) {}

    fn flush(&mut self) {}
}

impl HotstuffStorage {
    // TODO: remove qcs
    pub fn new(view: ViewNumber, init_node: &TreeNode, init_qc: &GenericQC) -> Self
    where
        Self: Sized,
    {
        Self {
            backend: todo!(),
            state: InMemoryState {
                current_view: view,
                vheight: view,
                committed_height: 0,
                b_executed: Arc::new(init_node.clone()),
                b_locked: Arc::new(init_node.clone()),
                // safety related
                leaf: Arc::new(init_node.clone()),
                // height\view of the leaf.
                qc_high: Arc::new(init_qc.clone()),
            },
            tc_queue: VecDeque::with_capacity(8),
            prop_queue: VecDeque::with_capacity(8),
            justify_queue: VecDeque::with_capacity(8),
        }
    }

    pub fn flush(&mut self) {}
}

impl SafetyStorage for HotstuffStorage {
    fn flush(&mut self) -> hotstuff_rs::safety::machine::Result<()> {
        todo!()
    }

    fn append_new_node(&mut self, node: &TreeNode) {
        let h = TreeNode::hash(node);
        self.backend.insert(h, Arc::new(node.clone()));
    }

    fn append_new_qc(&mut self, qc: &GenericQC) {
        self.justify_queue.push_back(qc.clone());
    }

    // todo: refactor
    fn find_three_chain(&self, node: &TreeNode) -> Vec<Arc<TreeNode>> {
        let mut chain = Vec::with_capacity(3);
        if let Some(b3) = self.backend.get(node.justify().node_hash()) {
            chain.push(b3.clone());
            if let Some(b2) = self.backend.get(b3.justify().node_hash()) {
                chain.push(b2.clone());
                if let Some(b1) = self.backend.get(b2.justify().node_hash()) {
                    chain.push(b1.clone());
                }
            }
        }
        chain
    }

    /// Persistent
    fn update_leaf(&mut self, new_leaf: &TreeNode) {
        self.state.vheight = new_leaf.height();
        self.state.leaf = Arc::new(new_leaf.clone());
        self.backend
            .insert(TreeNode::hash(new_leaf), self.state.leaf.clone());
    }

    fn get_leaf(&self) -> Arc<TreeNode> {
        self.state.leaf.clone()
    }

    fn get_qc_high(&self) -> Arc<GenericQC> {
        self.state.qc_high.clone()
    }

    /// qc_high.node == qc_node.
    fn update_qc_high(&mut self, new_qc_node: &TreeNode, new_qc_high: &GenericQC) {
        if let Some(qc_node) = self.backend.get(self.get_qc_high().node_hash()) {
            if new_qc_node.height() > qc_node.height() {
                self.state.qc_high = Arc::new(new_qc_high.clone());
                // self.vheight = new_qc_node.height();
                self.update_leaf(new_qc_node);

                debug!("update qc-high(h={})", new_qc_node.height());
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
        while height_now > b.height() {
            if let Some(prev) = self.backend.get(&parent_hash) {
                height_now = prev.height();
                parent_hash = prev.parent_hash().clone();
            } else {
                break;
            }
        }
        parent_hash != TreeNode::hash(b)
    }

    fn get_node(&self, node_hash: &NodeHash) -> Option<Arc<TreeNode>> {
        self.backend
            .get(node_hash)
            .and_then(|node| Some(node.clone()))
    }

    fn get_locked_node(&self) -> Arc<TreeNode> {
        self.state.b_locked.clone()
    }

    // Persistent
    fn update_locked_node(&mut self, node: &TreeNode) {
        debug!("locked at node with height {}", node.height());
        self.state.b_locked = Arc::new(node.clone());
    }

    fn get_last_executed(&self) -> Arc<TreeNode> {
        self.state.b_executed.clone()
    }

    // Persistent
    fn update_last_executed_node(&mut self, node: &TreeNode) {
        self.state.b_executed = Arc::new(node.clone());
    }

    fn get_view(&self) -> ViewNumber {
        self.state.current_view
    }

    // Persistent
    fn increase_view(&mut self, new_view: ViewNumber) {
        self.state.current_view = ViewNumber::max(self.state.current_view, new_view);
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
        self.state.b_executed = Arc::new(to_commit.clone());

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

impl LivenessStorage for HotstuffStorage {
    fn append_tc(&mut self, tc: TimeoutCertificate) -> Result<ViewNumber, LivenessStorageErr> {
        todo!()
    }

    fn is_reach_threshold(&self, view: ViewNumber) -> bool {
        todo!()
    }

    fn update_qc_high(&mut self, qc: &GenericQC) -> Result<(), LivenessStorageErr> {
        todo!()
    }

    fn fetch_branch(
        &self,
        strategy: &BranchSyncStrategy,
    ) -> Result<BranchData, LivenessStorageErr> {
        todo!()
    }

    fn is_qc_node_exists(&mut self, qc: &GenericQC) -> bool {
        todo!()
    }

    fn get_locked_node(&mut self) -> &TreeNode {
        todo!()
    }

    fn get_leaf(&self) -> &TreeNode {
        todo!()
    }
}
