//! This module includes functions like:
//! - hashing proposal
//! - validating proposal
//! - threshold signature
//!
//!

use hs_data::{CombinedSign, Sign, SignID, SignKit, TreeNode, PK, SK};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// TODO
#[derive(Debug, Clone, Serialize, Deserialize, Error)]
pub enum SignErr {
    #[error("Need at least {0} signatures, only {1} offered")]
    InsufficientSigns(usize, usize),
}

pub trait Signaturer: Send + Sync {
    fn sign_id(&self) -> SignID;

    fn sign(&self, node: &TreeNode) -> SignKit;

    fn combine_partial_sign<'a>(
        &self,
        partial_signs: impl IntoIterator<Item = &'a SignKit>,
    ) -> Result<Box<CombinedSign>, SignErr>;

    fn validate_vote(&self, prop: &TreeNode, vote: &SignKit) -> bool;

    fn validate_qc(&self, qc_node: &TreeNode, combined_sign: &CombinedSign) -> bool;
}

#[derive(Clone)]
pub struct DefaultSignaturer {
    sign_id: SignID,
    pks: PK,
    sks: SK,
}

impl DefaultSignaturer {
    pub fn new(sign_id: usize, pks: PK, sks: SK) -> Self {
        DefaultSignaturer { sign_id, pks, sks }
    }
}

impl Signaturer for DefaultSignaturer {
    fn sign_id(&self) -> SignID {
        self.sign_id
    }

    fn sign(&self, node: &TreeNode) -> SignKit {
        let buf = node.to_be_bytes();
        SignKit::new(self.sign_id, self.sks.sign(&buf))
    }

    fn combine_partial_sign<'a>(
        &self,
        partial_signs: impl IntoIterator<Item = &'a SignKit>,
    ) -> Result<Box<CombinedSign>, SignErr> {
        // wrapper
        let voters = partial_signs
            .into_iter()
            .map(|kit| (*kit.sign_id(), kit.sign()))
            .collect::<Vec<_>>();
        let voter_num = voters.len();

        match self.pks.combine_signatures(voters) {
            Ok(s) => Ok(Box::new(s)),
            Err(_) => Err(SignErr::InsufficientSigns(self.pks.threshold(), voter_num)),
        }
    }

    fn validate_vote(&self, prop: &TreeNode, vote: &SignKit) -> bool {
        self.pks
            .public_key_share(vote.sign_id())
            .verify(vote.sign(), prop.to_be_bytes())
    }

    fn validate_qc(&self, qc_node: &TreeNode, combined_sign: &CombinedSign) -> bool {
        self.pks
            .public_key()
            .verify(combined_sign, qc_node.to_be_bytes())
    }
}
