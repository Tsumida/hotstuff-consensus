//! Storage for liveness evnet, message.
//!

use crate::data::{BranchData, BranchSyncStrategy, TimeoutCertificate};
use hotstuff_rs::data::{GenericQC, TreeNode, ViewNumber};
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
}

/// LivenessStorage should be able to verify data from ohter peers.
pub trait LivenessStorage {
    /// Idempotently appending new TimeoutCertificates.
    /// Return `true` if there are at least `n-f` TCs from different replicas,
    /// which means that view is timeout and pacemaker should only process messages with larger view.
    fn append_tc(&mut self, tc: &TimeoutCertificate) -> Result<ViewNumber, LivenessStorageErr>;

    /// Return `true` if there are at least `n-f` TCs from different replicas,
    /// which means that view is timeout and pacemaker should only process messages with larger view.
    fn is_reach_threshold(&self, view: ViewNumber) -> bool;

    /// Update QC-High (highest GenericQC this replica has ever seen)
    /// TODO: consider use TreeNode.
    fn update_qc_high(&mut self, qc: &GenericQC) -> Result<(), LivenessStorageErr>;

    /// Append branch data
    fn sync_branch(
        &mut self,
        strategy: &BranchSyncStrategy,
        branch: BranchData,
    ) -> Result<(), LivenessStorageErr>;

    fn fetch_branch(&self, strategy: &BranchSyncStrategy)
        -> Result<BranchData, LivenessStorageErr>;

    fn is_qc_node_exists(&mut self, qc: &GenericQC) -> bool;

    fn get_locked_node(&mut self) -> &TreeNode;

    fn get_qc_high(&self) -> &TreeNode;
}
