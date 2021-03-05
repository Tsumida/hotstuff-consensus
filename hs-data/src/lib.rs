pub mod msg;
pub mod qc;
pub mod sign_kit;
pub mod tree_node;
pub mod txn;

use std::{convert::TryInto, mem::transmute_copy};

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

pub fn combined_sign_from_vec_u8(v: Vec<u8>) -> CombinedSign {
    let buf: [u8; 96] = v.try_into().unwrap();
    CombinedSign::from_bytes(buf).unwrap()
}
