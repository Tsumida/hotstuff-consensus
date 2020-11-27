
use std::sync::Arc;

use tokio::sync::mpsc::{Sender}; 
use serde::{Serialize, Deserialize}; 

use crate::machine::basic::*; 

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot{    
    pub view: ViewNumber, 
    pub leader: Option<ReplicaID>, 
    pub threshold: usize, 
    pub node_num: usize, 
    pub leaf: String, 
    pub qc_high: String, 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context{
    pub from: ReplicaID, 
    pub view: ViewNumber, 
}

#[derive(Clone, Debug)]
pub enum InternalMsg{

    // restful api
    RequestSnapshot(Sender<(Context, Box<Snapshot>)>),

    // As replica, recv proposal from leader.  -> on_recv_proposal
    RecvProposal(Context, Arc<TreeNode>, Arc<GenericQC>, Sender<(Context, Box<TreeNode>, Box<SignKit>)>), 

    // As leader, recv sign(ACK) from other replicas. 
    RecvSign(Context, Arc<TreeNode>, Arc<SignKit>), 

    // TODO:
    // Pacemaker related. As replica, update qc_high.
    // Note that the replica may not receive the proposal until receive this msg. 
    RecvNewViewMsg(Context, Arc<TreeNode>, Arc<GenericQC>), 

    // TODO: 
    // pacemaker -> statemachine, and then send new view msg. 
    // leader_id, new_view, 
    ViewChange(ReplicaID, u64, Sender<(Arc<TreeNode>, Arc<GenericQC>)>), 

    // As leader, broadcast proposal. 
    Propose(Context, Arc<TreeNode>, Arc<GenericQC>), 

    // New transaction from client. 
    NewTx(Cmd),

    // TODO: duplicate, remove it.
    // NewLeader
    NewLeader(Context, ReplicaID), 
    // tell http server to stop; 
    Down, 
}

impl InternalMsg{
    pub fn from_rpc_response(resp: RpcResponse) -> Option<Self>{
        if resp.status != RpcStatus::Accept || resp.node.is_none() || resp.sign.is_none(){
            return None; 
        }
        Some(
            InternalMsg::RecvSign(
                resp.ctx, 
                Arc::new(*resp.node.unwrap()), 
                Arc::new(*resp.sign.unwrap()), 
            )
        )
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RpcType{
    NewProposal, 
    NewView, 
    NewLeader, 
    Status, 
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RpcStatus{
    Ok, 
    InvalidUsage,
    InternalError, 

    // new proposal related 
    Accept, 
    Reject, 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewTransaction{
    pub client_id: String, 
    pub cmd: String, 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest{
    pub ctx: Context, 
    pub msg_type: RpcType, 
    pub leader: Option<ReplicaID>, 
    pub prop: Option<Box<TreeNode>>, 
    pub qc: Option<Box<GenericQC>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse{
    pub ctx: Context, 
    pub status: RpcStatus, 
    pub msg_type: RpcType, 
    pub node: Option<Box<TreeNode>>,
    pub sign: Option<Box<SignKit>>, 
    pub snapshot: Option<Box<Snapshot>>, 
    pub qc: Option<Box<GenericQC>>, 
}

impl RpcResponse{
    pub fn accept(ctx: &Context, node: &TreeNode, sign: &SignKit) -> Self{
        RpcResponse{
            ctx: ctx.clone(), 
            status: RpcStatus::Accept, 
            msg_type: RpcType::NewProposal, 
            node: Some(Box::new(node.clone())),
            sign: Some(Box::new(sign.clone())), 
            snapshot: None, 
            qc: None, 
        }
    }

    pub fn reject(ctx: &Context) -> Self{
        RpcResponse{
            ctx: ctx.clone(), 
            status: RpcStatus::Reject, 
            msg_type: RpcType::NewProposal, 
            node: None,
            sign: None, 
            snapshot: None, 
            qc: None, 
        }
    }

    pub fn internal_err(ctx: &Context, msg_type: RpcType) -> Self{
        RpcResponse{
            ctx: ctx.clone(), 
            status: RpcStatus::InternalError, 
            msg_type, 
            node: None,
            snapshot: None, 
            sign: None, 
            qc: None, 
        }
    }

    pub fn invalid_usage(ctx: &Context, msg_type: RpcType) -> Self{
        RpcResponse{
            ctx: ctx.clone(), 
            status: RpcStatus::InvalidUsage, 
            msg_type, 
            node: None,
            snapshot: None, 
            sign: None, 
            qc: None, 
        }
    }

    pub fn status(ctx: &Context, ss: Option<Box<Snapshot>>) -> Self{
        RpcResponse{
            ctx: ctx.clone(), 
            status: RpcStatus::Ok, 
            msg_type: RpcType::Status, 
            snapshot: ss.and_then(|ss| Some(Box::new(*ss))), 
            node: None,
            sign: None, 
            qc: None, 
        }
    }

    pub fn new_view(ctx: &Context, node: &TreeNode, qc: &GenericQC) -> Self{
        RpcResponse{
            ctx: ctx.clone(), 
            status: RpcStatus::Ok,
            msg_type: RpcType::NewView, 
            snapshot: None, 
            node: Some(Box::new(node.clone())),
            sign: None, 
            qc: Some(Box::new(qc.clone())), 
        }
    }

    pub fn ok(ctx: &Context, msg_type: RpcType) -> Self{
        RpcResponse{
            ctx: ctx.clone(), 
            status: RpcStatus::Ok,
            msg_type, 
            snapshot: None, 
            node: None,
            sign: None, 
            qc: None,  
        }
    }
}