use super::basic::ViewNumber;
use crate::msg::Context;
use crate::safety::basic::{CombinedSign, ReplicaID, Sign, SignID, SignKit, TreeNode, PK, SK};
use std::collections::HashMap;
use thiserror::Error;

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

pub struct Voter {
    // crypto related
    view: ViewNumber,
    sign_id: SignID,
    pks: PK,
    sks: SK,
    voting_set: HashMap<ReplicaID, SignKit>,
    vote_decided: bool,
}

impl Voter {
    pub fn new(sign_id: SignID, pks: PK, sks: SK) -> Self {
        Self {
            view: 0,
            sign_id,
            pks,
            sks,
            voting_set: HashMap::new(),
            vote_decided: false,
        }
    }

    pub fn sign_id(&self) -> SignID {
        self.sign_id
    }

    pub fn sign(&self, node: &TreeNode) -> Box<Sign> {
        let buf = node.to_be_bytes();
        let s = self.sks.sign(&buf);

        Box::new(s)
    }

    pub fn combine_partial_sign(&mut self) -> Result<Box<CombinedSign>, VoteErr> {
        if self.vote_set_size() <= self.pks.threshold() {
            return Err(VoteErr::InsufficientSigns(
                self.pks.threshold() + 1,
                self.vote_set_size(),
            ));
        }
        self.decide();

        // wrapper
        let tmp = self
            .voting_set
            .values()
            .map(|kit| (*kit.sign_id() as usize, kit.sign()))
            .collect::<Vec<_>>();

        Ok(Box::new(self.pks.combine_signatures(tmp).unwrap()))
    }

    pub fn reset(&mut self, new_view: ViewNumber) {
        self.view = new_view;
        self.voting_set.clear();
        self.vote_decided = false;
    }

    pub fn add_vote(&mut self, ctx: &Context, sign: &SignKit) -> Result<(), VoteErr> {
        if ctx.view != self.view {
            return Err(VoteErr::IncompatibleVote(ctx.from.clone(), ctx.view));
        } else {
            match self.voting_set.insert(ctx.from.clone(), sign.clone()) {
                None => Ok(()),
                Some(_) => Err(VoteErr::DuplicateVote(ctx.from.clone())),
            }
        }
    }

    pub fn vote_set_size(&self) -> usize {
        self.voting_set.len()
    }

    // stop recv votes until next view.
    fn decide(&mut self) {
        self.vote_decided = true;
    }
}
