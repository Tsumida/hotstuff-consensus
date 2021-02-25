//! HotstuffStorage implementation based on sled.

use std::path::{Path, PathBuf};

use hotstuff_rs::{data::ViewNumber, safety::safety_storage::SafetyStorage};
use pacemaker::liveness_storage::LivenessStorage;
use sled::Db;

use crate::{HotstuffStorage, LivenessState};

pub struct SledHSS {
    db: Db,
    ls_tree: Option<sled::Tree>,
    ss_tree: Option<sled::Tree>,

    /// hotstuff state:
    /// - qc-high
    /// - vheight
    /// - locked
    /// - last_committed
    /// - current_view
    state_tree: Option<sled::Tree>,

    config: SledConfig,
}

pub struct SledConfig {
    path_ls: PathBuf,
    path_ss: PathBuf,
    path_state: PathBuf,
}

impl HotstuffStorage for SledHSS {}

impl LivenessStorage for SledHSS {
    fn append_tc(
        &mut self,
        tc: &pacemaker::data::TimeoutCertificate,
    ) -> Result<hotstuff_rs::data::ViewNumber, pacemaker::liveness_storage::LivenessStorageErr>
    {
        todo!()
    }

    fn is_reach_threshold(&self, view: hotstuff_rs::data::ViewNumber) -> bool {
        todo!()
    }

    fn update_qc_high(
        &mut self,
        qc: &hotstuff_rs::data::GenericQC,
    ) -> Result<(), pacemaker::liveness_storage::LivenessStorageErr> {
        todo!()
    }

    fn sync_branch(
        &mut self,
        strategy: &pacemaker::data::BranchSyncStrategy,
        branch: pacemaker::data::BranchData,
    ) -> Result<(), pacemaker::liveness_storage::LivenessStorageErr> {
        todo!()
    }

    fn fetch_branch(
        &self,
        strategy: &pacemaker::data::BranchSyncStrategy,
    ) -> Result<pacemaker::data::BranchData, pacemaker::liveness_storage::LivenessStorageErr> {
        todo!()
    }

    fn is_qc_node_exists(&mut self, qc: &hotstuff_rs::data::GenericQC) -> bool {
        todo!()
    }

    fn get_locked_node(&mut self) -> &hotstuff_rs::data::TreeNode {
        todo!()
    }

    fn get_leaf(&self) -> &hotstuff_rs::data::TreeNode {
        todo!()
    }
}

impl SafetyStorage for SledHSS {
    fn append_new_node(&mut self, node: &hotstuff_rs::data::TreeNode) {
        todo!()
    }

    fn get_node(
        &self,
        node_hash: &hotstuff_rs::data::NodeHash,
    ) -> Option<std::sync::Arc<hotstuff_rs::data::TreeNode>> {
        todo!()
    }

    fn find_three_chain(
        &self,
        node: &hotstuff_rs::data::TreeNode,
    ) -> Vec<std::sync::Arc<hotstuff_rs::data::TreeNode>> {
        todo!()
    }

    fn is_consecutive_three_chain(
        &self,
        chain: &Vec<impl AsRef<hotstuff_rs::data::TreeNode>>,
    ) -> bool {
        todo!()
    }

    fn is_conflicting(
        &self,
        a: &hotstuff_rs::data::TreeNode,
        b: &hotstuff_rs::data::TreeNode,
    ) -> bool {
        todo!()
    }

    fn get_qc_high(&self) -> std::sync::Arc<hotstuff_rs::data::GenericQC> {
        todo!()
    }

    fn update_qc_high(
        &mut self,
        qc_node: &hotstuff_rs::data::TreeNode,
        qc_high: &hotstuff_rs::data::GenericQC,
    ) {
        todo!()
    }

    fn get_leaf(&self) -> std::sync::Arc<hotstuff_rs::data::TreeNode> {
        todo!()
    }

    fn update_leaf(&mut self, new_leaf: &hotstuff_rs::data::TreeNode) {
        todo!()
    }

    fn get_locked_node(&self) -> std::sync::Arc<hotstuff_rs::data::TreeNode> {
        todo!()
    }

    fn update_locked_node(&mut self, node: &hotstuff_rs::data::TreeNode) {
        todo!()
    }

    fn get_last_executed(&self) -> std::sync::Arc<hotstuff_rs::data::TreeNode> {
        todo!()
    }

    fn update_last_executed_node(&mut self, node: &hotstuff_rs::data::TreeNode) {
        todo!()
    }

    fn get_view(&self) -> hotstuff_rs::data::ViewNumber {
        todo!()
    }

    fn increase_view(&mut self, new_view: hotstuff_rs::data::ViewNumber) {
        todo!()
    }

    fn commit(&mut self, node: &hotstuff_rs::data::TreeNode) {
        todo!()
    }

    fn get_vheight(&self) -> hotstuff_rs::data::ViewNumber {
        todo!()
    }

    fn update_vheight(
        &mut self,
        vheight: hotstuff_rs::data::ViewNumber,
    ) -> hotstuff_rs::data::ViewNumber {
        todo!()
    }

    fn hotstuff_status(&self) -> hotstuff_rs::safety::safety_storage::Snapshot {
        todo!()
    }
}
