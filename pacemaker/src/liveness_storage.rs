//! Storage for liveness evnet, message.
//!

use std::{collections::HashMap, sync::Arc};

use crate::data::{BranchData, BranchSyncStrategy, TimeoutCertificate};
use hs_data::{GenericQC, ReplicaID, TreeNode, ViewNumber};
use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum LivenessStorageErr {
    #[error("invalid time certificate with view = {0}")]
    InvalidTC(ViewNumber),

    #[error("Duplicated data")]
    DuplicatedData,

    #[error("Stale data with view = {0}, current view is {1}")]
    StaleData(ViewNumber, ViewNumber),

    #[error("Invalid branch data")]
    InvalidBranch(BranchData),

    #[error("Failed to stablize data")]
    FailedToFlush,
}

#[async_trait::async_trait]
/// LivenessStorage should be able to verify data from ohter peers.
pub trait LivenessStorage {
    async fn flush(&mut self) -> Result<(), LivenessStorageErr>;

    /// Idempotently appending new TimeoutCertificates.
    /// Return `true` if there are at least `n-f` TCs from different replicas,
    /// which means that view is timeout and pacemaker should only process messages with larger view.
    fn append_tc(&mut self, tc: TimeoutCertificate) -> Result<ViewNumber, LivenessStorageErr>;

    /// Return `true` if there are at least `n-f` TCs from different replicas,
    /// which means that view is timeout and pacemaker should only process messages with larger view.
    fn is_reach_threshold(&self, view: ViewNumber) -> bool;

    /// Procedure `UpdateQCHigh(qc'high)` in Algorithm 5. This method will update qc-high and leaf. New leaf is qc-high.node.
    fn update_qc_high(&mut self, node: &TreeNode, qc: &GenericQC)
        -> Result<(), LivenessStorageErr>;

    /// Fetch branch according strategy.
    /// - Using `Grow` strategy, the resposer returns branch `[grow_from, min(grow_from + batch_size, responser.leaf)]`
    fn fetch_branch(&self, strategy: &BranchSyncStrategy)
        -> Result<BranchData, LivenessStorageErr>;

    fn is_qc_node_exists(&mut self, qc: &GenericQC) -> bool;

    fn get_locked_node(&mut self) -> &TreeNode;

    /// Leaf carries qc-high
    fn get_leaf(&self) -> &TreeNode;

    fn increase_view(&mut self, new_view: ViewNumber);

    /// Return true if the number of new-view msgs is at least `n-f`.
    fn new_view_set(&mut self) -> &mut HashMap<ReplicaID, Arc<GenericQC>>;

    fn clean_new_view_set(&mut self);

    fn get_threshold(&mut self) -> usize;
}

#[async_trait::async_trait]
pub trait AsyncFlusher<E: std::error::Error + Sync + Send> {
    async fn async_flush(&mut self) -> Result<(), E>;
}
