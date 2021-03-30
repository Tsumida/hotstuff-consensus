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

#[test]
#[ignore = "tested"]
fn type_size() {
    macro_rules! type_sizes {
        ($x:ty, $($y:ty), +) => {

            type_sizes!($x);
            type_sizes!($($y),+);
        };

        ($x:ty) => {
            println!(
                "size of {} is {} Byte",
                stringify!($x),
                std::mem::size_of::<$x>()
            );
        }
    }

    type_sizes!(PK, SK, Sign, CombinedSign, GenericQC);
}

pub fn form_combined_sign(node: &TreeNode, sks: &Vec<(usize, SK)>, pks: &PK) -> CombinedSign {
    let node_bytes = node.to_be_bytes();
    let signs = sks
        .iter()
        .map(|(i, sk)| (*i, sk.sign(&node_bytes)))
        .collect::<Vec<(usize, Sign)>>();
    pks.combine_signatures(signs.iter().map(|(i, s)| (*i, s)))
        .unwrap()
}

pub fn form_chain<'a>(
    txn: impl IntoIterator<Item = &'a str>,
    vec_sks: &Vec<(usize, SK)>,
    pk_set: &PK,
) -> Vec<(NodeHash, TreeNode)> {
    let mut v = vec![];
    let mut height = 0;
    let mut parent_hash = INIT_NODE_HASH.clone();
    let mut parent_node = INIT_NODE.clone();
    for tx in txn {
        let combined_sign = form_combined_sign(&parent_node, vec_sks, pk_set);
        let justify = GenericQC::new(height, &parent_hash, &combined_sign);
        let (node, hash) =
            TreeNode::node_and_hash(vec![&Txn::new(tx)], height + 1, &parent_hash, &justify);
        v.push((hash.as_ref().clone(), node.as_ref().clone()));

        height += 1;
        parent_hash = *hash;
        parent_node = *node;
    }
    v
}

#[test]
#[ignore = "tested"]
fn test_threshold_sign() {
    // Test:
    //     combine(0, 1, 2) == combine(1, 2, 3)
    //
    let n = 4;
    let (sks, pks, vec_sks) = crate::sign_kit::threshold_sign_kit(n, (n << 1) / 3);

    let sign_0_3 = vec_sks
        .iter()
        .take(3)
        .map(|(i, sk)| (*i, sk.sign(1u64.to_be_bytes())))
        .collect::<Vec<(usize, Sign)>>();
    let sign_1_4 = vec_sks[1..]
        .iter()
        .take(3)
        .map(|(i, sk)| (*i, sk.sign(1u64.to_be_bytes())))
        .collect::<Vec<(usize, Sign)>>();

    assert_eq!(
        pks.combine_signatures(sign_0_3.iter().map(|(a, b)| (*a, b))),
        pks.combine_signatures(sign_1_4.iter().map(|(a, b)| (*a, b)))
    );
}
