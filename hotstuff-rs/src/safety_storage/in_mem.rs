use std::{collections::HashMap, sync::Arc};

use log::{debug, info};

use crate::msg::*;
use crate::safety::basic::*;
use crate::traits::*;

pub struct InMemoryStorage {
    // storage related
    node_pool: HashMap<NodeHash, Arc<TreeNode>>,
    qc_map: HashMap<QCHash, Arc<GenericQC>>,

    // safety related
    leaf: Arc<TreeNode>,
    // height\view of the leaf.
    vheight: ViewNumber,

    qc_high: Arc<GenericQC>,

    view: ViewNumber,

    commit_height: ViewNumber,
    b_executed: Arc<TreeNode>,
    b_locked: Arc<TreeNode>,
}

impl InMemoryStorage {
    pub fn new(
        node_pool: HashMap<NodeHash, Arc<TreeNode>>,
        qc_map: HashMap<QCHash, Arc<GenericQC>>,
        view: ViewNumber,
        init_node: &TreeNode,
        init_qc: &GenericQC,
    ) -> Self {
        Self {
            node_pool,
            qc_map,
            view,
            vheight: view,
            commit_height: 0,
            b_executed: Arc::new(init_node.clone()),
            b_locked: Arc::new(init_node.clone()),
            // safety related
            leaf: Arc::new(init_node.clone()),
            // height\view of the leaf.
            qc_high: Arc::new(init_qc.clone()),
        }
    }
}

impl SafetyStorage for InMemoryStorage {
    fn append_new_node(&mut self, node: &TreeNode) {
        let h = TreeNode::hash(node);
        self.node_pool.insert(h, Arc::new(node.clone()));
        self.vheight = u64::max(self.vheight, node.height);
    }

    fn append_new_qc(&mut self, qc: &GenericQC) {
        let h = GenericQC::hash(qc);
        self.qc_map.insert(h, Arc::new(qc.clone()));
    }

    fn find_parent(&self, node: &TreeNode) -> Option<Arc<TreeNode>> {
        self.node_pool
            .get(&node.parent)
            .and_then(|node| Some(node.clone()))
    }

    // node should be in node pool.
    fn find_qc_by_justify(&self, node_hash: &NodeHash) -> Option<Arc<GenericQC>> {
        self.node_pool
            .get(node_hash)
            .and_then(|node| self.qc_map.get(&node.justify))
            .and_then(|qc| Some(qc.clone()))
    }

    fn find_node_by_qc(&self, qc_hash: &QCHash) -> Option<Arc<TreeNode>> {
        self.qc_map
            .get(qc_hash)
            .and_then(|qc| self.node_pool.get(&qc.node))
            .and_then(|node| Some(node.clone()))
    }

    fn find_three_chain(&self, node: &TreeNode) -> Vec<Arc<TreeNode>> {
        let mut chain = Vec::with_capacity(3);
        if let Some(qc3) = self.qc_map.get(&node.justify) {
            if let Some(b3) = self.node_pool.get(&qc3.node) {
                chain.push(b3.clone());
                if let Some(qc2) = self.qc_map.get(&b3.justify) {
                    if let Some(b2) = self.node_pool.get(&qc2.node) {
                        chain.push(b2.clone());
                        if let Some(qc1) = self.qc_map.get(&b2.justify) {
                            if let Some(b1) = self.node_pool.get(&qc1.node) {
                                chain.push(b1.clone());
                            }
                        }
                    }
                }
            }
        }
        chain
    }

    fn update_leaf(&mut self, new_leaf: &TreeNode) {
        self.vheight = new_leaf.height;
        self.leaf = Arc::new(new_leaf.clone());
        self.node_pool
            .insert(TreeNode::hash(new_leaf), self.leaf.clone());
    }

    fn get_leaf(&self) -> Arc<TreeNode> {
        self.leaf.clone()
    }

    fn get_qc_high(&self) -> Arc<GenericQC> {
        self.qc_high.clone()
    }

    /// qc_high.node == qc_node.
    fn update_qc_high(&mut self, new_qc_node: &TreeNode, new_qc_high: &GenericQC) {
        let qc_high = self.get_qc_high();
        if let Some(qc_node) = self.find_node_by_qc(&GenericQC::hash(qc_high.as_ref())) {
            let pred = new_qc_node.height > qc_node.height;
            if pred {
                self.qc_high = Arc::new(new_qc_high.clone());
                self.vheight = new_qc_node.height;
                self.update_leaf(new_qc_node);
                debug!("update qc-high(h={})", qc_node.height);
            }
        }
    }

    fn is_conflicting(&self, a: &TreeNode, b: &TreeNode) -> bool {
        let (a, b) = if a.height >= b.height { (a, b) } else { (b, a) };

        // a.height >= b.height
        let mut node = a;
        while node.height > b.height {
            if let Some(prev) = self.node_pool.get(&node.parent) {
                node = prev.as_ref();
            } else {
                break;
            }
        }

        TreeNode::hash(&node) != TreeNode::hash(b)
    }

    fn get_node(&self, node_hash: &NodeHash) -> Option<Arc<TreeNode>> {
        self.node_pool
            .get(node_hash)
            .and_then(|node| Some(node.clone()))
    }

    fn get_qc(&self, qc_hash: &QCHash) -> Option<Arc<GenericQC>> {
        self.qc_map.get(qc_hash).and_then(|qc| Some(qc.clone()))
    }

    fn get_locked_node(&self) -> Arc<TreeNode> {
        self.b_locked.clone()
    }

    fn update_locked_node(&mut self, node: &TreeNode) {
        debug!("locked at node with height {}", node.height);
        self.b_locked = Arc::new(node.clone());
    }

    fn get_last_executed(&self) -> Arc<TreeNode> {
        self.b_executed.clone()
    }

    fn update_last_executed_node(&mut self, node: &TreeNode) {
        self.b_executed = Arc::new(node.clone());
    }

    fn get_view(&self) -> ViewNumber {
        self.view
    }

    fn increase_view(&mut self, new_view: ViewNumber) {
        self.view = ViewNumber::max(self.view, new_view);
    }

    fn is_consecutive_three_chain(&self, chain: &Vec<impl AsRef<TreeNode>>) -> bool {
        if chain.len() != 3 {
            debug!("not consecutive 3-chain, len={}", chain.len());
            return false;
        }

        let b_3 = chain.get(0).unwrap().as_ref();
        let b_2 = chain.get(1).unwrap().as_ref();
        let b = chain.get(2).unwrap().as_ref();

        let pred_32 = &b_3.parent == &TreeNode::hash(b_2);
        let pred_21 = &b_2.parent == &TreeNode::hash(b);
        // &b_3.parent == &TreeNode::hash(b_2) && &b_2.parent == &TreeNode::hash(b)
        debug!(
            "consecutive judge with h = {},{},{}: {} - {}",
            b_3.height, b_2.height, b.height, pred_32, pred_21
        );
        pred_32 && pred_21
    }

    fn get_vheight(&self) -> ViewNumber {
        self.vheight
    }

    // TODO: add informer for watchers.
    fn commit(&mut self, to_commit: &TreeNode) {
        if self.commit_height >= to_commit.height {
            debug!("to_commit with smaller height {}", to_commit.height);
            return;
        }
        let to_commit_height = to_commit.height;
        for h in self.commit_height + 1..=to_commit_height {
            // TODO:execute,
            self.commit_height = h;
        }
        self.b_executed = Arc::new(to_commit.clone());
        info!(
            "commit new proposal, commit_height = {}",
            self.commit_height
        );
    }

    fn hotstuff_status(&self) -> Snapshot {
        Snapshot {
            view: self.view,
            leader: None,
            qc_high: Box::new(self.qc_high.as_ref().clone()),
            leaf: Box::new(self.leaf.as_ref().clone()),
            locked_node: Box::new(self.b_locked.as_ref().clone()),
            last_committed: self.commit_height,
        }
    }
}
