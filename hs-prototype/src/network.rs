//! Network component which offer quorum abstractions. +
use tokio::{
    
};
use crate::basic::{
    ViewNumber, 
    TreeNode, 
};
/// Quorum-based consensus system 
pub struct NetworkComponent{
}

impl NetworkComponent{
    /// As leader, broadcast new proposal and wait for responses, 
    /// NetworkComponent stop the quorum when received timeout msg from Pacemaker. 
    pub async fn broadcast_proposal(&mut self, view: ViewNumber, proposal: Box<TreeNode>){

    }

    /// As leader, recv a partial sign from replica. 
    pub async fn recv_response(&mut self){}

    // As Replica, respond to leader with sign. 
    pub async fn resp_response(&mut self){}
}