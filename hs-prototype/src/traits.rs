use serde::{
    Serialize, Deserialize, 
};
use std::sync::Arc; 
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
    pub to: ReplicaID, 
    pub view: ViewNumber, 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest{
    ctx: Context, 
    msg_type: RequestType, 
    // proposal 
    node: Option<Box<TreeNode>>, 
    // new view msg
    qc: Option<Box<GenericQC>>, 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResponseType{
    Accpet,     // accept proposal, 
    Vote,       // will to vote. 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse{
    ctx: Context, 
    msg_type: ResponseType, 
    sign: Option<Box<SignKit>>, 
    will_to_vote: bool, 
}

pub trait HotStuff: SysConf + StateMachine + Pacemaker + Crypto{
    fn is_leader(&self) -> bool;

    // Return or ignore if self is not the leader. 
    fn on_recv_vote(&mut self, ctx: &Context, node: &TreeNode, sign: &SignKit); 

    // Return immediately if self is not the leader. 
    fn on_recv_proposal(&mut self, ctx: &Context, node: &TreeNode); 

    fn on_beat(&mut self, cmds: &Vec<Cmd>);
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

    fn safe_node(&mut self, node: &TreeNode, qc: &GenericQC) -> bool; 

}

pub trait Timer{
    fn reset_timer(&mut self);

    fn tick(&mut self, delta: u64); 

    fn deadline(&self) -> u64; 
    
    fn update_deadline(&mut self, deadline: u64); 

    fn touch_deadline(&self) -> bool; 
}

pub trait Pacemaker: MemPool + Timer{
    /// leader election; 
    fn leader_election(&mut self);

    fn view_change(&mut self); 
}

pub trait MemPool{
    // Append new node into mempool. 
    fn append_new_node(&mut self, node: &TreeNode);

    fn append_new_qc(&mut self, qc: &GenericQC); 

    fn get_node(&mut self, node_hash: &NodeHash) -> Option<Arc<TreeNode>>; 

    // if node is genesis, return None. 
    fn find_parent(&self, node: &TreeNode) -> Option<Arc<TreeNode>>;

    // Get GenericQC by node.justify
    fn find_qc_by_justify(&self, node_hash: &NodeHash) -> Option<Arc<GenericQC>>;

    // Get node through GenericQC.node
    fn find_node_by_qc(&self, qc_hash: &QCHash) -> Option<Arc<TreeNode>>;

    // b'', b', b
    fn find_three_chain(&self, node:&NodeHash) -> Vec<Arc<TreeNode>>;

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

pub trait Network{
    // new round 
    fn propose(&mut self, node: &TreeNode); 

    // As replica, accept and reply. 
    fn accept_proposal(&mut self, ctx: &Context, node:&TreeNode, sign: &SignKit); 

    // Broadcast information about new leader. 
    fn new_leader(&mut self, ctx: &Context, leader: &ReplicaID); 
}
