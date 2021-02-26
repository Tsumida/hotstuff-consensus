use serde::{Deserialize, Serialize};
use std::sync::Arc;

use hs_data::{CombinedSign, GenericQC, NodeHash, QCHash, Sign, SignID, TreeNode, ViewNumber};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RequestType {
    NewView,
    Proposal,
    Vote,
}

pub trait SysConf {
    fn self_id(&self) -> &str;

    fn get_addr(&self, node_id: &String) -> Option<&String>;

    fn threshold(&self) -> usize;
}

pub trait Crypto {
    fn sign_id(&self) -> SignID;

    fn sign(&self, node: &TreeNode) -> Box<Sign>;

    // TODO: return Result
    fn combine_partial_sign(&self) -> Box<CombinedSign>;
}

pub trait Timer {
    fn reset_timer(&mut self);

    fn tick(&mut self, delta: u64);

    fn deadline(&self) -> u64;

    fn update_deadline(&mut self, deadline: u64);

    fn touch_deadline(&self) -> bool;
}
