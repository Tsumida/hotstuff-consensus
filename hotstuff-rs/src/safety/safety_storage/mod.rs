pub mod in_mem;

use crate::data::*;

use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub trait SafetyStorage {
    // Append new node
    fn append_new_node(&mut self, node: &TreeNode);

    // fn append_new_qc(&mut self, qc: &GenericQC);
    fn get_node(&self, node_hash: &NodeHash) -> Option<Arc<TreeNode>>;

    // fn get_qc(&self, qc_hash: &QCHash) -> Option<Arc<GenericQC>>;

    // if node is genesis, return None.
    // fn find_parent(&self, node: &TreeNode) -> Option<Arc<TreeNode>>;

    // Get GenericQC by node.justify, and node should be in node pool already.
    // fn find_qc_by_justify(&self, node_hash: &NodeHash) -> Option<Arc<GenericQC>>;

    // Get node through GenericQC.node, and qc should be in node pool already.
    // fn find_node_by_qc(&self, qc_hash: &QCHash) -> Option<Arc<TreeNode>>;

    // b'', b', b
    fn find_three_chain(&self, node: &TreeNode) -> Vec<Arc<TreeNode>>;

    fn is_consecutive_three_chain(&self, chain: &Vec<impl AsRef<TreeNode>>) -> bool;

    fn is_conflicting(&self, a: &TreeNode, b: &TreeNode) -> bool;

    fn get_qc_high(&self) -> Arc<GenericQC>;

    fn update_qc_high(&mut self, qc_node: &TreeNode, qc_high: &GenericQC);

    fn get_leaf(&self) -> Arc<TreeNode>;

    // Check height before update leaf.
    fn update_leaf(&mut self, new_leaf: &TreeNode);

    fn get_locked_node(&self) -> Arc<TreeNode>;

    /// Update locked node
    fn update_locked_node(&mut self, node: &TreeNode);

    fn get_last_executed(&self) -> Arc<TreeNode>;

    fn update_last_executed_node(&mut self, node: &TreeNode);

    fn get_view(&self) -> ViewNumber;

    fn increase_view(&mut self, new_view: ViewNumber);

    fn commit(&mut self, node: &TreeNode);

    // Get height of last voted node.
    fn get_vheight(&self) -> ViewNumber;

    // return previous viewnumber.
    fn update_vheight(&mut self, vheight: ViewNumber) -> ViewNumber;

    fn hotstuff_status(&self) -> Snapshot;
}

/// Snapshot for machine's internal state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub view: ViewNumber,
    pub leader: Option<ReplicaID>,
    pub qc_high: Box<GenericQC>,
    pub leaf: Box<TreeNode>,
    pub locked_node: Box<TreeNode>,
    pub last_committed: ViewNumber,
}
