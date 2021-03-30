use std::{collections::HashMap, sync::Arc};

use hs_data::msg::*;
use hs_data::*;

use log::{debug, error, info};
use thiserror::Error;

use super::{voter::VoteErr, voter::Voter};
use cryptokit::DefaultSignaturer;

pub type Result<T> = core::result::Result<T, SafetyErr>;

use serde::{Deserialize, Serialize};

/// SafetyStorage
#[async_trait::async_trait]
pub trait SafetyStorage {
    async fn flush(&mut self) -> Result<()>;

    /// Append new node and the qc it carries. Use this method if and only if node is verified and the qc it carries is valid.
    fn append_new_node(&mut self, node: &TreeNode);

    /// Append a valid qc only.
    /// Use this method even when a rejected proposal carrying a valid qc is encountered.
    fn append_new_qc(&mut self, qc: &GenericQC);

    /// fn append_new_qc(&mut self, qc: &GenericQC);
    fn get_node(&self, node_hash: &NodeHash) -> Option<Arc<TreeNode>>;

    /// Get three-chain `b'' -> b' -> b`. The first node is `b''`.
    fn find_three_chain(&self, node: &TreeNode) -> Vec<Arc<TreeNode>>;

    /// Return True if `b.parent is b' and b'.parent is b'' `. Return false if `chain.len() != 3`.
    fn is_consecutive_three_chain(&self, chain: &Vec<impl AsRef<TreeNode>>) -> bool;

    fn is_conflicting(&self, a: &TreeNode, b: &TreeNode) -> bool;

    fn get_qc_high(&self) -> Arc<GenericQC>;

    fn update_qc_high(&mut self, qc_node: &TreeNode, qc_high: &GenericQC);

    fn get_leaf(&self) -> Arc<TreeNode>;

    // Update leaf and stablize new leaf.
    fn update_leaf(&mut self, new_leaf: &TreeNode);

    fn get_locked_node(&self) -> Arc<TreeNode>;

    /// Update locked node
    fn update_locked_node(&mut self, node: &TreeNode);

    fn get_view(&self) -> ViewNumber;

    fn increase_view(&mut self, new_view: ViewNumber);

    fn commit(&mut self, node: &TreeNode);

    fn get_last_committed(&self) -> Arc<TreeNode>;

    // Get height of last voted node.
    fn get_vheight(&self) -> ViewNumber;

    // return previous viewnumber.
    fn update_vheight(&mut self, vheight: ViewNumber) -> ViewNumber;

    fn hotstuff_status(&self) -> Snapshot;
}

/// Snapshot for machine's internal state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub view: ViewNumber,
    pub leader: Option<ReplicaID>,
    pub qc_high: Box<GenericQC>,
    pub leaf: Box<TreeNode>,
    pub locked_node: Box<TreeNode>,
    pub last_committed: ViewNumber,
}

#[derive(Clone, Debug)]
pub enum SafetyEvent {
    // restful api
    RequestSnapshot,

    // As replica, recv proposal from leader.  -> on_recv_proposal
    RecvProposal(Context, Arc<TreeNode>),

    // As leader, recv sign(ACK) from other replicas.
    RecvSign(Context, Arc<TreeNode>, Box<SignKit>),

    RecvNewViewMsg(Context, Arc<GenericQC>),

    // pacemaker -> statesafety, and then send new view msg.
    // leader_id, new_view,
    ViewChange(ReplicaID, u64),

    // As leader, broadcast proposal.
    Propose(Context, Arc<TreeNode>, Arc<GenericQC>),

    // New transaction from client.
    NewTx(Vec<Txn>),

    // TODO: remove it.
    // NewLeader
    NewLeader(Context, ReplicaID),

    // Branch synchronizing
    BranchSync(Context, Vec<TreeNode>),
}

#[derive(Debug)]
pub enum Ready {
    // nothing to do
    Nil,
    //
    InternalState(Context, Snapshot),
    // Form new proposal.
    NewProposal(Context, Arc<TreeNode>),
    // Node in this message carries qc-high
    UpdateQCHigh(Context, Arc<TreeNode>),
    // Signature for the proposal
    Signature(Context, Arc<TreeNode>, Box<SignKit>),
    // TODO: remove
    CommitState(Context, Arc<TreeNode>),

    BranchSyncDone(Arc<TreeNode>),

    // This msg carries view for lasted proposal formed qc.
    ProposalReachConsensus(ViewNumber),
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
    fn on_beat(&mut self, cmds: Vec<Txn>) -> Result<Ready>;

    /// commit nodes and return the latest state about commitment.
    fn on_commit(&mut self, node: &TreeNode) -> Result<Ready>;

    fn on_view_change(&mut self, leader: ReplicaID, view: ViewNumber) -> Result<Ready>;

    fn on_branch_sync(&mut self, branch: Vec<TreeNode>) -> Result<Ready>;

    fn update_nodes(&mut self, node: &TreeNode) -> Result<Ready>;

    fn safe_node(&mut self, node: &TreeNode, prev_node: &TreeNode) -> bool;

    fn take_snapshot(&self) -> Snapshot;

    fn process_safety_event(&mut self, req: SafetyEvent) -> Result<Ready>;
}

#[derive(Debug, Error)]
pub enum SafetyErr {
    #[error("invalid view number {0}")]
    InvalidViewNumber(u64),

    #[error("invalid leader {0}")]
    InvalidLeader(ReplicaID),

    #[error("error in voting: {0}")]
    VoterError(VoteErr),

    #[error("corrupted vote")]
    CorruptedVote,

    // Note that for case init <- a1 <- a2 <- a3, all of them were proposed by correct leader.
    // if we didn't recv a2 before a3 carrying qc of a2,
    // we actually can't recongnize whether this qc is corrupted or correct.
    #[error("corrupted qc")]
    CorruptedQC,

    #[error("Internal storage error")]
    InternalErr(Box<dyn std::error::Error>),
}

pub struct Machine<S: SafetyStorage> {
    // view: ViewNumber,
    storage: S,
    voter: Voter<DefaultSignaturer>,

    // config related
    total: usize,
    self_id: ReplicaID,
    leader_id: Option<ReplicaID>,
    //input: Receiver<SafetyEvent>,
    //output: Sender<SafetyEvent>,
}

impl<S: SafetyStorage> Safety for Machine<S> {
    /// See Algorithm 5 in the paper.
    /// As replica, recv proposal from leader.
    fn update_nodes(&mut self, node: &TreeNode) -> Result<Ready> {
        let chain = self.storage.find_three_chain(node);
        let mut ready = Ready::Nil;
        // debug!("find chain with {} nodes", chain.len());
        if let Some(b_3) = chain.get(0) {
            self.storage.update_qc_high(b_3.as_ref(), node.justify());
        }

        if let Some(b_2) = chain.get(1) {
            let locked = self.storage.get_locked_node();
            if b_2.height() > locked.height() {
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
        Ok(Ready::CommitState(
            Context::response(self.self_id.clone(), self.storage.get_view()),
            self.storage.get_last_committed(),
        ))
    }

    fn safe_node(&mut self, node: &TreeNode, justify_node: &TreeNode) -> bool {
        //
        //  non-conflict
        //
        //          a <--- .... <--- b
        //
        //  conflicting
        //               <--- b
        //              /
        //      <--- c <----- a
        //
        let locked = self.storage.get_locked_node();
        let conflicting = self.storage.is_conflicting(node, &locked);

        let b = justify_node.height() > locked.height();

        debug!(
            "safe_node() for node with h = {}: {} - {}",
            node.height(),
            !conflicting,
            b
        );
        !conflicting || b
    }

    // Make new proposal. Note that leader will sign it first.
    fn on_beat(&mut self, cmds: Vec<Txn>) -> Result<Ready> {
        let prop = self.make_leaf(&cmds);
        self.storage.update_leaf(&prop);

        let ctx = Context::broadcast(self.self_id.clone(), self.storage().get_view());

        let sign_kit = self.voter.sign(&prop);
        let _ = self.voter.add_vote(&ctx, prop.height(), &sign_kit);

        info!(
            "make new proposal with height = {}, justify={}",
            prop.height(),
            prop.justify().view()
        );
        Ok(Ready::NewProposal(ctx, Arc::new(*prop)))
    }

    // TODO: add validating.
    fn on_recv_vote(&mut self, ctx: &Context, prop: &TreeNode, sign: &SignKit) -> Result<Ready> {
        if !self.voter.validate_vote(prop, sign) {
            return Err(SafetyErr::CorruptedVote);
        }
        if let Err(e) = self.voter.add_vote(ctx, prop.height(), sign) {
            error!("{:?}", e);
            return Ok(Ready::Nil);
        }

        info!("recv vote from {}, prop.view={}", &ctx.from, prop.height());
        // vote at most once.
        if self.voter.vote_set_size(prop.height()) > self.threshold() {
            match self.voter.combine_partial_sign(prop.height()) {
                // TODO: leaf as prop <=> no new proposal
                Ok(combined_sign) => {
                    let prop_hash = TreeNode::hash(self.storage().get_leaf().as_ref());
                    let qc = GenericQC::new(
                        // TODO: should be node.height()?
                        self.storage.get_view(),
                        &prop_hash,
                        combined_sign.as_ref(),
                    );
                    self.storage.update_qc_high(&prop, &qc);
                    info!("qc formed");

                    Ok(Ready::ProposalReachConsensus(qc.view()))
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
        info!("recv proposal with h = {}", prop.height());
        let mut ready = self.verify_proposal(justify)?;

        if let Some(prev_node) = self.storage.get_node(prop.justify().node_hash()) {
            if prop.height() > self.storage.get_vheight()
                && self.safe_node(prop, prev_node.as_ref())
            {
                // Implicitly append qc.
                self.storage.append_new_node(&prop);
                self.storage.update_vheight(prop.height());
                // sign
                let sign_kit = self.voter.sign(&prop);

                ready = Ready::Signature(
                    Context::single(
                        self.self_id.clone(),
                        ctx.from.clone(),
                        self.storage().get_view(),
                    ),
                    Arc::new(prop.clone()),
                    Box::new(sign_kit),
                );
            }
        } else {
            // The qc is verified, so we take it.
            self.storage.append_new_qc(prop.justify());
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
            self.form_update_qc_high()
        } else {
            Ready::Nil
        };

        Ok(ready)
    }

    // TODO: let storage do  job.
    fn take_snapshot(&self) -> Snapshot {
        let mut ss = self.storage.hotstuff_status();
        ss.leader = self.leader_id.clone();
        ss
    }

    fn process_safety_event(&mut self, req: SafetyEvent) -> Result<Ready> {
        match req {
            SafetyEvent::RequestSnapshot => {
                let ss = self.take_snapshot();
                Ok(Ready::InternalState(
                    Context::broadcast(self.self_id.clone(), self.storage().get_view()),
                    ss,
                ))
            }
            SafetyEvent::RecvProposal(ctx, proposal) => {
                self.on_recv_proposal(&ctx, proposal.as_ref(), proposal.as_ref().justify())
            }
            SafetyEvent::RecvSign(ctx, node, sign) => {
                self.on_recv_vote(&ctx, node.as_ref(), sign.as_ref())
            }
            SafetyEvent::NewTx(cmds) => self.on_beat(cmds),
            SafetyEvent::RecvNewViewMsg(ctx, qc_high) => {
                // update qc-high
                match self.storage.get_node(qc_high.node_hash()) {
                    Some(qc_node) => {
                        self.storage.update_qc_high(&qc_node, &qc_high);
                    }
                    _ => error!("can't find qc_high.node for new-view msg from {}", ctx.from),
                }

                Ok(self.form_update_qc_high())
            }
            // TODO: remove
            SafetyEvent::NewLeader(ctx, leader) => {
                if ctx.view >= self.storage.get_view() && self.leader_id.is_none() {
                    self.leader_id = Some(leader);
                }
                Ok(Ready::Nil)
            }
            // SafetyEvent::ViewChange(leader, view) => self.on_view_change(leader, view),
            SafetyEvent::BranchSync(_, branch) => self.on_branch_sync(branch),
            _ => {
                error!("recv invalid msg");
                Ok(Ready::Nil)
            }
        }
    }

    fn on_branch_sync(&mut self, branch: Vec<TreeNode>) -> Result<Ready> {
        info!(
            "recv branch sync data: {:?}",
            branch
                .iter()
                .map(|prop| prop.height())
                .collect::<Vec<ViewNumber>>()
        );
        for prop in branch {
            if !self.verify_proposal(&prop.justify()).is_ok() {
                break;
            }
            if let Some(prev_node) = self.storage.get_node(prop.justify().node_hash()) {
                if prop.height() > self.storage.get_vheight()
                    && self.safe_node(&prop, prev_node.as_ref())
                {
                    self.storage.append_new_node(&prop);
                    self.storage.update_vheight(prop.height());
                }
            };
            let _ = self.update_nodes(&prop);
        }

        Ok(Ready::BranchSyncDone(self.storage.get_leaf()))
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

    fn form_update_qc_high(&self) -> Ready {
        // Note
        // leaf.qc == qc-high
        Ready::UpdateQCHigh(
            Context::response(self.self_id.clone(), self.storage.get_view()),
            self.storage.get_leaf(),
        )
    }

    fn verify_proposal(&self, justify: &GenericQC) -> Result<Ready> {
        if let Some(qc_node) = self.storage.get_node(justify.node_hash()) {
            // any correct qc has combined signature except for init_qc.
            if !GenericQC::is_init_qc(&justify) {
                let valid = self.voter.validate_qc(&qc_node, justify.combined_sign());
                debug!(
                    "recv prop validate: {} with justify.view = {}",
                    valid,
                    justify.view()
                );
                if !valid {
                    return Err(SafetyErr::CorruptedQC);
                }
            }
            Ok(Ready::Nil)
        } else {
            debug!("prop.justify.node(h={}) not found", justify.view());
            Err(SafetyErr::CorruptedQC)
        }
    }

    fn make_leaf(&self, cmds: &Vec<Txn>) -> Box<TreeNode> {
        let prev_leaf = self.storage.get_leaf();
        let parent = TreeNode::hash(prev_leaf.as_ref());
        let justify = self.storage.get_qc_high();
        let (node, _) = TreeNode::node_and_hash(cmds, self.storage.get_view(), &parent, &justify);
        node
    }

    pub fn new(
        voter: Voter<DefaultSignaturer>,
        self_id: String,
        total: usize,
        leader_id: Option<String>,
        storage: S,
    ) -> Self {
        Self {
            voter,
            self_id,
            total,
            leader_id,
            storage,
        }
    }

    pub fn storage(&mut self) -> &mut S {
        &mut self.storage
    }
}
