use cryptokit::{SignErr, Signaturer};
use hs_data::{msg::Context, CombinedSign, ReplicaID, SignKit, TreeNode, ViewNumber};

use std::collections::{BTreeMap, HashMap};
use thiserror::Error;

use log::error;

#[derive(Debug, Clone, Error)]
pub enum VoteErr {
    #[error("recv duplicate vote from {0}")]
    DuplicateVote(ReplicaID),

    #[error("recv incompatible vote with view={1} from {0}")]
    IncompatibleVote(ReplicaID, ViewNumber),

    #[error("voting in view={0} is decided")]
    RoundDecided(ViewNumber),

    #[error("Need at least {0} signatures, only {1} offered")]
    InsufficientSigns(usize, usize),
}

pub struct Voter<S: Signaturer> {
    // crypto related
    view: ViewNumber,
    // signaturer related
    signature: S,
    threshold: usize,
    voting_set: BTreeMap<ViewNumber, HashMap<ReplicaID, SignKit>>,
    // TODO: use vheight
    vote_decided: bool,
}

impl<S: Signaturer> Voter<S> {
    pub fn new(threshold: usize, signature: S) -> Self {
        Self {
            view: 0,
            // TODO:
            signature,
            threshold,
            voting_set: BTreeMap::new(),
            vote_decided: false,
        }
    }

    pub fn sign(&self, node: &TreeNode) -> SignKit {
        self.signature.sign(node)
    }

    /// Consume the voting set and generate combined signature.
    pub fn combine_partial_sign(&mut self, view: ViewNumber) -> Result<Box<CombinedSign>, VoteErr> {
        if self.vote_set_size(view) <= self.threshold {
            return Err(VoteErr::InsufficientSigns(
                self.threshold + 1,
                self.vote_set_size(view),
            ));
        }
        self.decide();

        let res = self
            .signature
            .combine_partial_sign(self.voting_set.get_mut(&view).unwrap().values())
            .map_err(|SignErr::InsufficientSigns(n, m)| VoteErr::InsufficientSigns(n, m));
        let _ = self.voting_set.remove(&view);
        res
    }

    pub fn validate_vote(&self, prop: &TreeNode, vote: &SignKit) -> bool {
        self.signature.validate_vote(prop, vote)
    }

    pub fn validate_qc(&self, qc_node: &TreeNode, combined_sign: &CombinedSign) -> bool {
        self.signature.validate_qc(qc_node, combined_sign)
    }

    pub fn reset(&mut self, new_view: ViewNumber) {
        self.view = new_view;
        self.remove_stale_vote(new_view);
        self.vote_decided = false;
    }

    pub fn add_vote(
        &mut self,
        ctx: &Context,
        view: ViewNumber,
        sign: &SignKit,
    ) -> Result<(), VoteErr> {
        match self
            .voting_set
            .entry(view)
            .or_insert(HashMap::new())
            .insert(ctx.from.clone(), sign.clone())
        {
            None => Ok(()),
            Some(_) => Err(VoteErr::DuplicateVote(ctx.from.clone())),
        }
    }

    pub fn vote_set_size(&self, view: ViewNumber) -> usize {
        self.voting_set.get(&view).map_or(0, |x| x.len())
    }

    // Clear voting set and stop recv votes until next view.
    // There won't be another voting set with at least n-f votes with view=current_view.
    fn decide(&mut self) {
        self.vote_decided = true;
    }

    #[inline]
    pub fn remove_stale_vote(&mut self, new_view: ViewNumber) {
        let new_voting_set = self.voting_set.split_off(&new_view);
        self.voting_set = new_voting_set;
    }
}
