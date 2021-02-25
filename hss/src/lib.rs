//! Hotstuff storage.

pub mod sled_hss;

use hotstuff_rs::{
    data::{TreeNode, ViewNumber},
    safety::safety_storage::SafetyStorage,
};
use pacemaker::liveness_storage::LivenessStorage;

pub trait HotstuffStorage: SafetyStorage + LivenessStorage {}

#[derive(Debug, Clone)]
pub struct LivenessState {
    node_with_qc_high: TreeNode,
    leaf_hight: ViewNumber,
    current_view: ViewNumber,
}

#[derive(Debug, Clone)]
pub struct SafetyState {
    vheight: ViewNumber,
    last_executed: ViewNumber,
}

pub trait PersistentStorage {
    // flush all dirty data into disk.
    fn flush(&mut self) {}
}
