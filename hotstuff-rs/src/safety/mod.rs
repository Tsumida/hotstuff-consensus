use std::sync::mpsc::{Receiver, Sender};
use std::{collections::HashMap, sync::Arc};

use log::{debug, error, info};
use thiserror::Error;

pub mod basic;
pub mod voter;
use crate::msg::Context;
use crate::msg::*;
use crate::traits::*;
use basic::*;
use voter::Voter;

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
    NewTx(Cmds),

    // TODO: duplicate, remove it.
    // NewLeader
    NewLeader(Context, ReplicaID),
}

#[derive(Clone, Debug)]
pub enum Ready {
    Nil,
    InternalState(Context, Box<Snapshot>),
    NewProposal(Context, Arc<TreeNode>, Arc<GenericQC>),
    UpdateQCHigh(Context, Arc<TreeNode>, Arc<GenericQC>),
    Signature(Context, Arc<TreeNode>, Box<SignKit>),
    CommitState(Context, ViewNumber),
}

pub trait Safety {
    fn is_leader(&self) -> bool;

    // Return or ignore if self is not the leader.
    fn on_recv_vote(&mut self, ctx: &Context, node: &TreeNode, sign: &SignKit) -> Ready;

    // Return immediately if self is not the leader.
    fn on_recv_proposal(&mut self, ctx: &Context, node: &TreeNode, justify: &GenericQC) -> Ready;

    fn on_beat(&mut self, proposal: &TreeNode) -> Ready;

    // commit nodes and return the latest state about commitment
    fn on_commit(&mut self, node: &TreeNode) -> Ready;

    fn on_view_change(&mut self, leader: ReplicaID, view: ViewNumber) -> Ready;

    fn update_nodes(&mut self, node: &TreeNode) -> Ready;

    fn safe_node(&mut self, node: &TreeNode, prev_node: &TreeNode) -> bool;

    // fn propose(&mut self, node: &TreeNode, qc_high: Arc<GenericQC>);
}

#[derive(Debug, Error)]
pub enum SafetyErr {
    #[error("invalid view number {0}")]
    InvalidViewNumber(u64),

    #[error("invalid leader {0}")]
    InvalidLeader(ReplicaID),
}

pub struct Machine<S: SafetyStorage> {
    // view: ViewNumber,
    storage: S,
    voter: Voter,

    // config related
    peer_conf: HashMap<ReplicaID, String>,
    self_id: ReplicaID,
    leader_id: Option<ReplicaID>,
    //input: Receiver<SafetyEvent>,
    //output: Sender<SafetyEvent>,
}

impl<S: SafetyStorage> Safety for Machine<S> {
    /// As replica, recv proposal from leader.
    /// Note that
    fn update_nodes(&mut self, node: &TreeNode) -> Ready {
        let chain = self.storage.find_three_chain(node);
        let b_lock = self.storage.get_locked_node();
        let mut ready = Ready::Nil;

        // debug!("find chain with {} nodes", chain.len());

        if let Some(b_3) = chain.get(0) {
            let b3_qc = self.storage.get_qc(&node.justify).unwrap();
            self.storage.update_qc_high(b_3.as_ref(), b3_qc.as_ref());
        }

        if let Some(b_2) = chain.get(1) {
            if b_2.height > b_lock.height {
                self.storage.update_locked_node(node);
            }
        }

        if chain.len() == 3 && self.storage.is_continues_three_chain(&chain) {
            ready = self.on_commit(chain.last().unwrap());
        }

        ready
    }

    fn on_commit(&mut self, node: &TreeNode) -> Ready {
        let b_exec = self.storage.get_locked_node();
        if b_exec.height < node.height {
            self.storage.commit(node);
        }
        let s = self.storage.storage_state();
        Ready::CommitState(self.get_context(), s.commit_height)
    }

    // TODO: unit test
    fn safe_node(&mut self, node: &TreeNode, prev_node: &TreeNode) -> bool {
        let a = !self
            .storage
            .is_conflicting(node, self.storage.get_locked_node().as_ref());
        // TODO: in paper b_new.jusitfy.node.height
        let b = prev_node.height > self.storage.get_locked_node().height;
        debug!("{} safe_node() result {} - {}", &self.self_id, a, b);
        a || b
    }

    fn is_leader(&self) -> bool {
        self.leader_id
            .as_ref()
            .map_or(false, |leader| leader == &self.self_id)
    }

    // start prposal
    fn on_beat(&mut self, proposal: &TreeNode) -> Ready {
        info!("{} beats", self.self_id);
        self.storage.update_leaf(proposal);
        Ready::Nil
    }

    // TODO: seperate hashing.
    // Return or ignore if self is not the leader.
    fn on_recv_vote(&mut self, ctx: &Context, prop: &TreeNode, sign: &SignKit) -> Ready {
        if let Err(e) = self.voter.add_vote(ctx, sign) {
            error!("{:?}", e);
            return Ready::Nil;
        }

        info!("recv partial sign from {}", &ctx.from);
        // vote at most once.
        if self.voter.vote_set_size() > self.threshold() {
            //self.compute_combined_sign();
            let combined_sign = self.voter.combine_partial_sign();
            // TODO: leaf as prop <=> no new proposal
            let prop_hash = TreeNode::hash(prop);
            let qc = GenericQC {
                view: self.storage.get_view(),
                node: prop_hash,
                combined_sign: Some(*combined_sign),
            };
            self.storage.update_qc_high(&prop, &qc);
            self.storage.append_new_qc(&qc);
            info!("qc formed");
        }
        Ready::Nil
    }

    // Return immediately if self is not the leader.
    fn on_recv_proposal(&mut self, ctx: &Context, prop: &TreeNode, justify: &GenericQC) -> Ready {
        self.storage.append_new_qc(justify);
        let ready = if let Some(prev_node) = self.storage.find_node_by_qc(&prop.justify) {
            if prop.height > self.storage.get_leaf_height()
                && self.safe_node(prop, prev_node.as_ref())
            {
                self.storage.append_new_node(&prop);
                let kit = SignKit::from((*self.voter.sign(prop), self.voter.sign_id()));
                Ready::Signature(
                    Context {
                        view: self.storage.get_view(),
                        from: self.self_id.clone(),
                    },
                    Arc::new(prop.clone()),
                    Box::new(kit),
                )
            } else {
                Ready::Nil
            }
        } else {
            Ready::Nil
        };
        self.update_nodes(prop);
        ready
    }

    fn on_view_change(&mut self, leader: ReplicaID, view: ViewNumber) -> Ready {
        if view > self.storage.get_view() {
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
        }
    }
}

impl<S: SafetyStorage> Machine<S> {
    /// Threshold of the size of quorum set.
    /// Suppose hotstuff has n nodes:
    /// - n = 3k,   threshold = 2k,   so there are atmost k-1 faulty nodes.
    /// - n = 3k+1, threshold = 2k,   so there are atmost k faulty nodes.
    /// - n = 3k+2, threshold = 2k+1, so there are atmost k faulty nodes
    #[inline(always)]
    fn threshold(&self) -> usize {
        (self.peer_conf.len() << 1) / 3
    }

    fn get_context(&self) -> Context {
        Context {
            from: self.self_id.clone(),
            view: self.storage.get_view(),
        }
    }

    fn make_leaf(&self, cmds: &Vec<Cmd>) -> Box<TreeNode> {
        let prev_leaf = self.storage.get_leaf();
        let parent = TreeNode::hash(prev_leaf.as_ref());
        let justify = GenericQC::hash(self.storage.get_qc_high().as_ref());
        let (node, _) = TreeNode::node_and_hash(cmds, self.storage.get_view(), &parent, &justify);
        node
    }

    /*
    fn propose(&mut self, node: &TreeNode, qc: Arc<GenericQC>) {
        self.output
            .send(SafetyEvent::Propose(
                Context {
                    view: self.storage.get_view(),
                    from: self.self_id.clone(),
                },
                Arc::new(node.clone()),
                qc,
            ))
            .unwrap();
    }*/

    fn take_snapshot(&self) -> Box<Snapshot> {
        let ss = Box::new(Snapshot {
            view: self.storage.get_view(),
            leader: self.leader_id.clone(),
            threshold: self.threshold(),
            leaf: base64::encode(&self.storage.get_last_executed().as_ref().to_be_bytes()),
            qc_high: base64::encode(&self.storage.get_qc_high().as_ref().to_be_bytes()),
        });
        ss
    }

    pub fn process_safety_event(&mut self, req: SafetyEvent) -> Ready {
        match req {
            SafetyEvent::RequestSnapshot => {
                let ss = self.take_snapshot();
                Ready::InternalState(self.get_context(), ss)
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
                Ready::UpdateQCHigh(self.get_context(), qc_node, self.storage.get_qc_high())
            }
            // TODO: remove
            SafetyEvent::NewLeader(ctx, leader) => {
                if ctx.view >= self.storage.get_view() && self.leader_id.is_none() {
                    self.leader_id = Some(leader);
                }
                Ready::Nil
            }
            SafetyEvent::ViewChange(leader, view) => self.on_view_change(leader, view),
            _ => {
                error!("recv invalid msg");
                Ready::Nil
            }
        }
    }
}
