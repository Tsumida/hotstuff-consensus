use super::{qc::QCHash, txn::Txn};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    ) -> Box<TreeNode> {
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
