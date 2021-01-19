mod qc;
mod sign_kit;
mod tree_node;
mod txn;

use serde::{Deserialize, Serialize};

pub use qc::*;
pub use sign_kit::*;
pub use tree_node::*;
pub use txn::*;

pub type PK = threshold_crypto::PublicKeySet;
pub type SK = threshold_crypto::SecretKeyShare;
pub type Sign = threshold_crypto::SignatureShare;
pub type CombinedSign = threshold_crypto::Signature;
pub type ReplicaID = String;
pub type ViewNumber = u64;
pub type SignID = usize;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum Role {
    Leader,
    Follower,
}
