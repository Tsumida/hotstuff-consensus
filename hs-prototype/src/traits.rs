
use std::sync::Arc; 
use serde::{
    Serialize, Deserialize, 
};
use tokio::sync::mpsc::{Sender, Receiver};
use crate::basic::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RequestType{
    NewView, 
    Proposal, 
    Vote, 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context{
    pub from: ReplicaID, 
    pub view: ViewNumber, 
}

#[async_trait::async_trait]
pub trait HotStuff: SysConf + StateMachine + Crypto{
    fn is_leader(&self) -> bool;

    // Return or ignore if self is not the leader. 
    async fn on_recv_vote(&mut self, ctx: &Context, node: &TreeNode, sign: &SignKit); 

    // Return immediately if self is not the leader. 
    async fn on_recv_proposal(&mut self, ctx: &Context, node: &TreeNode, justify:&GenericQC, sender: Sender<(Context, Box<TreeNode>, Box<SignKit>)>); 

    async fn on_beat(&mut self, cmds: &Vec<Cmd>);

    async fn propose(&mut self, node: &TreeNode, qc_high: Arc<GenericQC>); 

}

pub trait SysConf{
    fn self_id(&self) -> &str;

    fn get_addr(&self, node_id: &String) -> Option<&String>;

    fn threshold(&self) -> usize;
}

pub trait Crypto{
    fn sign_id(&self) -> SignID; 

    fn sign(&self, node:&TreeNode) -> Box<Sign>; 
    
    // TODO: return Result
    fn combine_partial_sign(&self) -> Box<CombinedSign>; 
}

pub trait StateMachine: MemPool{
    fn on_commit(&mut self, node: &TreeNode);

    fn update_nodes(&mut self, node: &TreeNode); 

    fn safe_node(&mut self, node: &TreeNode, prev_node: &TreeNode) -> bool; 

}

pub trait Timer{
    fn reset_timer(&mut self);

    fn tick(&mut self, delta: u64); 

    fn deadline(&self) -> u64; 
    
    fn update_deadline(&mut self, deadline: u64); 

    fn touch_deadline(&self) -> bool; 
}

#[async_trait::async_trait]
pub trait Pacemaker{
    /// Start leader election.
    async fn leader_election(&mut self);

    async fn view_change(&mut self); 
}

pub trait MemPool{
    // Append new node into mempool. 
    fn append_new_node(&mut self, node: &TreeNode);

    fn append_new_qc(&mut self, qc: &GenericQC); 

    fn get_node(&self, node_hash: &NodeHash) -> Option<Arc<TreeNode>>; 

    fn get_qc(&self, qc_hash: &QCHash) -> Option<Arc<GenericQC>>; 

    // if node is genesis, return None. 
    fn find_parent(&self, node: &TreeNode) -> Option<Arc<TreeNode>>;

    // Get GenericQC by node.justify, and node should be in node pool already. 
    fn find_qc_by_justify(&self, node_hash: &NodeHash) -> Option<Arc<GenericQC>>;

    // Get node through GenericQC.node, and qc should be in node pool already.
    fn find_node_by_qc(&self, qc_hash: &QCHash) -> Option<Arc<TreeNode>>;

    // b'', b', b
    fn find_three_chain(&self, node: &TreeNode) -> Vec<Arc<TreeNode>>;

    fn is_continues_three_chain(&self, chain: &Vec<impl AsRef<TreeNode>>) -> bool; 

    fn is_conflicting(&self, a:&TreeNode, b: &TreeNode) -> bool;

    fn get_qc_high(&self) -> Arc<GenericQC>;

    fn update_qc_high(&mut self, qc_node: &TreeNode, qc_high: &GenericQC);

    fn get_leaf(&self) -> Arc<TreeNode>;

    fn update_leaf(&mut self, new_leaf: &TreeNode);

    fn get_locked_node(&self) -> Arc<TreeNode>; 

    fn update_locked_node(&mut self, node: &TreeNode);

    fn get_last_executed(&self) -> Arc<TreeNode>; 

    fn update_last_executed_node(&mut self, node: &TreeNode); 

    fn get_view(&self) -> ViewNumber;

    fn increase_view(&mut self, new_view: ViewNumber);

    // Reset view related status like voting set
    fn reset(&mut self); 

    // false means duplicate signs from the same replica. 
    fn add_vote(&mut self, ctx: &Context, sign: &SignKit) -> bool;

    fn vote_set_size(&self) -> usize; 
}
