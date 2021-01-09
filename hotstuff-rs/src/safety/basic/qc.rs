use serde::{Deserialize, Serialize};

use super::{CombinedSign, NodeHash, TreeNode, ViewNumber};
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

    // TODO: consider corrupted qc with None as combined_sign.
    #[inline(always)]
    pub fn is_init_qc(qc: &GenericQC) -> bool {
        qc.combined_sign.is_none()
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
