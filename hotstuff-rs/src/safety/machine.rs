use std::sync::Arc;

use log::{debug, error, info};
use thiserror::Error;

use super::voter::Voter;
use super::{basic::*, voter};
use crate::{msg::Context};
use crate::msg::*;
use crate::traits::*;

pub type Result<T> = core::result::Result<T, SafetyErr>;

#[derive(Clone, Debug)]
pub enum SafetyEvent {
    // restful api
    RequestSnapshot,

    // As replica, recv proposal from leader.  -> on_recv_proposal
    RecvProposal(Context, Arc<TreeNode>, Arc<GenericQC>),

    // As leader, recv sign(ACK) from other replicas.
    RecvSign(Context, Arc<TreeNode>, Arc<SignKit>),

    // TODO:
    // Pacemaker related. As replica, update qc_high.
    // Note that the replica may not receive the proposal until receive this msg.
    RecvNewViewMsg(Context, Arc<TreeNode>, Arc<GenericQC>),

    // TODO:
    // pacemaker -> statesafety, and then send new view msg.
    // leader_id, new_view,
    ViewChange(ReplicaID, u64),

    // As leader, broadcast proposal.
    Propose(Context, Arc<TreeNode>, Arc<GenericQC>),

    // New transaction from client.
    NewTx(Vec<Txn>),

    // TODO: duplicate, remove it.
    // NewLeader
    NewLeader(Context, ReplicaID),
}

#[derive(Debug)]
pub enum Ready {
    // nothing to do
    Nil,
    //
    InternalState(Context, Box<Snapshot>),
    // New proposal and it's justify.
    NewProposal(Context, Arc<TreeNode>, Arc<GenericQC>),
    //
    UpdateQCHigh(Context, Arc<TreeNode>, Arc<GenericQC>),
    // Signature for the proposal
    Signature(Context, Arc<TreeNode>, Box<SignKit>),
    // TODO: remove
    CommitState(Context, ViewNumber),
}

/// Safety defines replica's reaction to message from other hotstuff peers. 
/// Any Message received must be validated before further processing. 
pub trait Safety {
    // As leader, recv vote from a certain replica. Return `Ready::Nil` if the vote is valid. 
    fn on_recv_vote(&mut self, ctx: &Context, node: &TreeNode, sign: &SignKit) -> Result<Ready>;

    /// Reaction to a new proposal. 
    /// Return `Ready::Nil` for rejecting, otherwise return `Ready::Signature` for accepting. 
    fn on_recv_proposal(
        &mut self,
        ctx: &Context,
        node: &TreeNode,
        justify: &GenericQC,
    ) -> Result<Ready>;

    /// Form new proposal for queueing transactions. 
    fn on_beat(&mut self, proposal: &TreeNode) -> Result<Ready>;

    /// commit nodes and return the latest state about commitment. 
    fn on_commit(&mut self, node: &TreeNode) -> Result<Ready>;

    fn on_view_change(&mut self, leader: ReplicaID, view: ViewNumber) -> Result<Ready>;

    fn update_nodes(&mut self, node: &TreeNode) -> Result<Ready>;

    fn safe_node(&mut self, node: &TreeNode, prev_node: &TreeNode) -> bool;

    fn take_snapshot(&self) -> Box<Snapshot>;

    fn process_safety_event(&mut self, req: SafetyEvent) -> Result<Ready>;
}

#[derive(Debug, Error)]
pub enum SafetyErr {
    #[error("invalid view number {0}")]
    InvalidViewNumber(u64),

    #[error("invalid leader {0}")]
    InvalidLeader(ReplicaID),

    #[error("error in voting: {0}")]
    VoterError(voter::VoteErr),
    
    #[error("corrupted vote")]
    CorruptedVote, 

    // Note that for case init <- a1 <- a2 <- a3, all of them were proposed by correct leader. 
    // if we didn't recv a2 before a3 carrying qc of a2, 
    // we actually can't recongnize whether this qc is corrupted or correct. 
    #[error("corrupted qc")]
    CorruptedQC, 
}

pub struct Machine<S: SafetyStorage> {
    // view: ViewNumber,
    storage: S,
    voter: Voter,

    // config related
    total: usize, 
    self_id: ReplicaID,
    leader_id: Option<ReplicaID>,
    //input: Receiver<SafetyEvent>,
    //output: Sender<SafetyEvent>,
}

impl<S: SafetyStorage> Safety for Machine<S> {
    /// As replica, recv proposal from leader.
    /// Note that
    fn update_nodes(&mut self, node: &TreeNode) -> Result<Ready> {
        let chain = self.storage.find_three_chain(node);
        let locked = self.storage.get_locked_node();
        let mut ready = Ready::Nil;

        // debug!("find chain with {} nodes", chain.len());

        if let Some(b_3) = chain.get(0) {
            let b3_qc = self.storage.get_qc(&node.justify).unwrap();
            self.storage.update_qc_high(b_3.as_ref(), b3_qc.as_ref());
        }

        if let Some(b_2) = chain.get(1) {
            if b_2.height > locked.height {
                self.storage.update_locked_node(b_2.as_ref());
            }
        }

        if self.storage.is_consecutive_three_chain(&chain) {
            ready = self.on_commit(chain.last().unwrap())?;
        }

        Ok(ready)
    }

    fn on_commit(&mut self, to_commit: &TreeNode) -> Result<Ready> {
        self.storage.commit(to_commit);
        Ok(Ready::Nil)
    }

    fn safe_node(&mut self, node: &TreeNode, justify_node: &TreeNode) -> bool {
        let locked = self.storage.get_locked_node(); 
        let conflicting = self
            .storage
            .is_conflicting(node, &locked);

        let b = justify_node.height > locked.height;
        
        debug!("safe_node() for node with h = {}: {} - {}", &node.height, !conflicting, b);
        !conflicting || b
    }

    // start prposal
    fn on_beat(&mut self, proposal: &TreeNode) -> Result<Ready> {
        info!("{} beats", self.self_id);
        self.storage.update_leaf(proposal);
        Ok(Ready::Nil)
    }

    // TODO: add validating. 
    fn on_recv_vote(&mut self, ctx: &Context, prop: &TreeNode, sign: &SignKit) -> Result<Ready> {
        if !self.voter.validate_vote(prop, sign){
            return Err(SafetyErr::CorruptedVote);
        }
        if let Err(e) = self.voter.add_vote(ctx, sign) {
            error!("{:?}", e);
            return Ok(Ready::Nil);
        }

        info!("recv partial sign from {}", &ctx.from);
        // vote at most once.
        if self.voter.vote_set_size() > self.threshold() {
            //self.compute_combined_sign();
            match self.voter.combine_partial_sign() {
                // TODO: leaf as prop <=> no new proposal
                Ok(combined_sign) => {
                    let prop_hash = TreeNode::hash(prop);
                    let qc = GenericQC {
                        // TODO: should be node.height?
                        view: self.storage.get_view(),
                        node: prop_hash,
                        combined_sign: Some(*combined_sign),
                    };
                    self.storage.update_qc_high(&prop, &qc);
                    self.storage.append_new_qc(&qc);
                    info!("qc formed");
                    Ok(Ready::Nil)
                }
                Err(e) => Err(SafetyErr::VoterError(e)),
            }
        } else {
            Ok(Ready::Nil)
        }
    }

    // TODO: add validating
    fn on_recv_proposal(
        &mut self,
        ctx: &Context,
        prop: &TreeNode,
        justify: &GenericQC,
    ) -> Result<Ready> {
        if let Some(qc_node) = self.storage.get_node(&justify.node){
            // any correct qc has combined signature except for init_qc.
            if !GenericQC::is_init_qc(&justify){
                let valid = self.voter.validate_qc(&qc_node, justify.combined_sign.as_ref().unwrap()); 
                debug!("recv prop validate: {}",valid); 
                if !valid {
                    return Err(SafetyErr::CorruptedQC);
                }
            }
        }else{
            debug!("prop.justify.node not found"); 
            return Err(SafetyErr::CorruptedQC);
        }

        self.storage.append_new_qc(justify);
        info!("recv proposal with h = {}", prop.height); 
                
        let mut ready = Ready::Nil;
        if let Some(prev_node) = self.storage.find_node_by_qc(&prop.justify) {
            if prop.height > self.storage.get_vheight()
                && self.safe_node(prop, prev_node.as_ref())
            {
                self.storage.append_new_node(&prop);
                // sign
                let kit = SignKit::from((*self.voter.sign(prop), self.voter.sign_id()));
                
                ready = Ready::Signature(
                    Context {
                        view: self.storage.get_view(),
                        from: self.self_id.clone(),
                    },
                    Arc::new(prop.clone()),
                    Box::new(kit),
                );
            }
        };
        let _ = self.update_nodes(prop);
        Ok(ready)
    }

    fn on_view_change(&mut self, leader: ReplicaID, view: ViewNumber) -> Result<Ready> {
        let ready = if view > self.storage.get_view() {
            self.leader_id = Some(leader);
            self.voter.reset(view);
            self.storage.increase_view(view);
            debug!("view change {}", self.storage.get_view());
            Ready::UpdateQCHigh(
                self.get_context(),
                self.storage.get_leaf(),
                self.storage.get_qc_high(),
            )
        } else {
            Ready::Nil
        };

        Ok(ready)
    }

    // TODO: let storage do  job. 
    fn take_snapshot(&self) -> Box<Snapshot> {
        let mut ss = self.storage.hotstuff_status(); 
        ss.as_mut().leader = self.leader_id.clone(); 
        ss
    }

    fn process_safety_event(&mut self, req: SafetyEvent) -> Result<Ready> {
        match req {
            SafetyEvent::RequestSnapshot => {
                let ss = self.take_snapshot();
                Ok(Ready::InternalState(self.get_context(), ss))
            }
            SafetyEvent::RecvProposal(ctx, proposal, justify) => {
                self.on_recv_proposal(&ctx, proposal.as_ref(), justify.as_ref())
            }
            SafetyEvent::RecvSign(ctx, node, sign) => {
                self.on_recv_vote(&ctx, node.as_ref(), sign.as_ref())
            }
            SafetyEvent::NewTx(cmds) => {
                // TODO: use mem pool
                // let cmds = vec![cmd];
                let node = self.make_leaf(&cmds);
                self.on_beat(&node)
            }
            SafetyEvent::RecvNewViewMsg(_, _, qc_high) => {
                // note: recv largest qc_high
                let qc_node = self.storage.get_node(&qc_high.node).unwrap();
                self.storage
                    .update_qc_high(qc_node.as_ref(), qc_high.as_ref());
                Ok(Ready::UpdateQCHigh(
                    self.get_context(),
                    qc_node,
                    self.storage.get_qc_high(),
                ))
            }
            // TODO: remove
            SafetyEvent::NewLeader(ctx, leader) => {
                if ctx.view >= self.storage.get_view() && self.leader_id.is_none() {
                    self.leader_id = Some(leader);
                }
                Ok(Ready::Nil)
            }
            SafetyEvent::ViewChange(leader, view) => self.on_view_change(leader, view),
            _ => {
                error!("recv invalid msg");
                Ok(Ready::Nil)
            }
        }
        
    }
}

impl<S: SafetyStorage> Machine<S> {
    /// Threshold of the size of quorum set.
    /// Suppose the hotstuff consists of n replicas:
    /// - n = 3k,   threshold = 2k    -> atmost k-1 faulty nodes.
    /// - n = 3k+1, threshold = 2k    -> atmost k faulty nodes.
    /// - n = 3k+2, threshold = 2k+1  -> atmost k faulty nodes
    /// # Example
    #[inline(always)]
    fn threshold(&self) -> usize {
        (self.total << 1) / 3
    }

    fn get_context(&self) -> Context {
        Context {
            from: self.self_id.clone(),
            view: self.storage.get_view(),
        }
    }

    fn make_leaf(&self, cmds: &Vec<Txn>) -> Box<TreeNode> {
        let prev_leaf = self.storage.get_leaf();
        let parent = TreeNode::hash(prev_leaf.as_ref());
        let justify = GenericQC::hash(self.storage.get_qc_high().as_ref());
        let (node, _) = TreeNode::node_and_hash(cmds, self.storage.get_view(), &parent, &justify);
        node
    }

    pub fn new(voter: Voter, self_id: String, total: usize, leader_id: Option<String>, storage: S) -> Self{
        Self{
            voter,
            self_id, 
            total, 
            leader_id, 
            storage, 
        }
    }

    
}
