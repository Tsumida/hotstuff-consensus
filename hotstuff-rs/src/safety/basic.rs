use sha2::{Digest, Sha256};
use std::{hash::Hash, ops::Deref};

use serde::{Deserialize, Serialize};

pub type PK = threshold_crypto::PublicKeySet;
pub type SK = threshold_crypto::SecretKeyShare;
pub type Sign = threshold_crypto::SignatureShare;
pub type CombinedSign = threshold_crypto::Signature;
pub type ReplicaID = String;
pub type ViewNumber = u64;
pub type SignID = usize;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Txn(Vec<u8>);

impl AsRef<[u8]> for Txn {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Txn {
    pub fn new(cont: impl AsRef<[u8]>) -> Self {
        Txn(cont.as_ref().to_vec())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SignKit {
    sign: Sign,
    sign_id: SignID,
}

impl SignKit {
    #[inline]
    pub fn sign(&self) -> &Sign {
        &self.sign
    }
    #[inline]
    pub fn sign_id(&self) -> &SignID {
        &self.sign_id
    }
}

impl std::convert::From<(Sign, SignID)> for SignKit {
    fn from(v: (Sign, SignID)) -> Self {
        Self {
            sign: v.0,
            sign_id: v.1,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum Role {
    Leader,
    Follower,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeHash(pub [u8; 32]);

impl NodeHash {
    pub fn genesis() -> NodeHash {
        NodeHash([0xAB; 32])
    }
}

impl std::convert::AsRef<[u8]> for NodeHash {
    fn as_ref(&self) -> &[u8] {
        return &self.0;
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QCHash(pub [u8; 32]);

impl std::convert::AsRef<[u8]> for QCHash {
    fn as_ref(&self) -> &[u8] {
        return &self.0;
    }
}

impl QCHash {
    pub fn genesis() -> QCHash {
        QCHash([0; 32])
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenericQC {
    // view equals node.height
    pub view: u64,
    pub node: NodeHash,
    // None for bootstrapping.
    pub combined_sign: Option<CombinedSign>,
}

impl GenericQC {
    pub fn new(view: u64, node: &NodeHash, combined_sign: &CombinedSign) -> Self {
        GenericQC {
            view,
            node: node.clone(),
            combined_sign: Some(combined_sign.clone()),
        }
    }

    pub fn genesis(view: ViewNumber, node: &TreeNode) -> Self {
        GenericQC {
            view,
            node: TreeNode::hash(node),
            combined_sign: None,
        }
    }

    // TODO: refactor
    pub fn hash(&self) -> QCHash {
        let mut res = [0u8; 32];
        if let Some(ref v) = self.combined_sign {
            res.copy_from_slice(&v.to_bytes()[32..64]);
        }
        // None -> [0u8; 32]
        QCHash(res)
    }

    pub fn to_be_bytes(&self) -> Vec<u8> {
        let size =
            8 + std::mem::size_of::<NodeHash>() + if self.combined_sign.is_some() { 96 } else { 0 };

        let mut buf = Vec::with_capacity(size);
        buf.extend_from_slice(&self.view.to_be_bytes());
        buf.extend_from_slice(&self.node.as_ref());
        if self.combined_sign.is_some() {
            buf.extend_from_slice(&self.combined_sign.as_ref().unwrap().to_bytes());
        }

        buf
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeNode {
    pub height: u64, // height === viewNumber
    // pub txs: Vec<Cmd>,
    pub txs: Vec<Txn>,
    pub parent: NodeHash,
    pub justify: QCHash,
}

impl TreeNode {
    pub fn genesis() -> TreeNode {
        TreeNode {
            height: 0,
            txs: vec![],
            parent: NodeHash::genesis(),
            justify: QCHash::genesis(),
        }
    }

    /// Hash using sha256.
    pub fn hash(node: &TreeNode) -> NodeHash {
        let mut h = Sha256::default();
        for s in &node.txs {
            h.update(s);
        }
        h.update(node.height.to_be_bytes());
        h.update(node.parent.0);

        let mut res = [0; 32];
        res.copy_from_slice(&h.finalize());
        NodeHash(res)
    }

    pub fn new<'a>(
        txs: impl IntoIterator<Item = &'a Txn>,
        height: u64,
        parent: &NodeHash,
        justify: &QCHash,
    ) -> Box<TreeNode>{
        let node = Box::new(TreeNode {
            txs: txs.into_iter().cloned().collect::<Vec<Txn>>(),
            height,
            parent: parent.clone(),
            justify: justify.clone(),
        });

        let hash = Box::new(TreeNode::hash(&node));

        node
    }

    pub fn node_and_hash<'a>(
        txs: impl IntoIterator<Item = &'a Txn>,
        height: u64,
        parent: &NodeHash,
        justify: &QCHash,
    ) -> (Box<TreeNode>, Box<NodeHash>) {
        let node = Box::new(TreeNode {
            txs: txs.into_iter().cloned().collect::<Vec<Txn>>(),
            height,
            parent: parent.clone(),
            justify: justify.clone(),
        });

        let hash = Box::new(TreeNode::hash(&node));

        (node, hash)
    }

    pub fn to_be_bytes(&self) -> Vec<u8> {
        let mut size = std::mem::size_of_val(&self.height);
        size += self.justify.as_ref().len();
        size += self.parent.as_ref().len();
        size += self.txs.iter().fold(0, |n, s| n + s.as_ref().len());

        let mut buf = Vec::with_capacity(size);
        buf.extend_from_slice(&self.height.to_be_bytes());
        buf.extend_from_slice(self.justify.as_ref());
        buf.extend_from_slice(self.parent.as_ref());
        for c in &self.txs {
            buf.extend_from_slice(c.as_ref());
        }
        buf
    }
}
