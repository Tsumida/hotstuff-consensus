//! This module includes functions like:
//! - hashing proposal
//! - validating proposal
//! - threshold signature
//!
//!

use crate::data::{CombinedSign, Sign, SignID, SignKit, TreeNode};
use serde::{Deserialize, Serialize};
// TODO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SignErr {}

pub trait Signaturer {
    fn sign_id(&self) -> SignID;

    fn sign(&self, node: &TreeNode) -> Box<Sign>;

    fn combine_partial_sign(&mut self) -> Result<Box<CombinedSign>, SignErr>;

    fn validate_vote(&self, prop: &TreeNode, vote: &SignKit) -> bool;

    fn validate_qc(&self, qc_node: &TreeNode, combined_sign: &CombinedSign) -> bool;
}
