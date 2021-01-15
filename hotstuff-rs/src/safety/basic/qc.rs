use super::INIT_NODE_HASH;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};

lazy_static! {
    pub static ref INIT_QC: GenericQC = GenericQC {
        view: 0,
        node: (*INIT_NODE_HASH).clone(),
        combined_sign: unsafe { std::mem::zeroed() },
    };
    pub static ref INIT_QC_HASH: QCHash = GenericQC::hash(&INIT_QC);
}
use super::{CombinedSign, NodeHash, ViewNumber};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenericQC {
    // view equals node.height
    view: u64,
    node: NodeHash,
    // None for bootstrapping.
    combined_sign: CombinedSign,
}

impl GenericQC {
    pub fn new(view: u64, node: &NodeHash, combined_sign: &CombinedSign) -> Self {
        GenericQC {
            view,
            node: node.clone(),
            combined_sign: combined_sign.clone(),
        }
    }

    // TODO: consider corrupted qc with None as combined_sign.
    #[inline(always)]
    pub fn is_init_qc(qc: &GenericQC) -> bool {
        GenericQC::hash(qc) == *INIT_QC_HASH
    }

    // TODO: refactor
    pub fn hash(&self) -> QCHash {
        let mut res = [0u8; 32];
        res.copy_from_slice(&self.combined_sign.to_bytes()[32..64]);
        // None -> [0u8; 32]
        QCHash(res)
    }

    pub fn to_be_bytes(&self) -> Vec<u8> {
        let size = 8 + std::mem::size_of::<NodeHash>() + std::mem::size_of::<CombinedSign>();

        let mut buf = Vec::with_capacity(size);
        buf.extend_from_slice(&self.view.to_be_bytes());
        buf.extend_from_slice(&self.node.as_ref());
        buf.extend_from_slice(&self.combined_sign.to_bytes());
        buf
    }

    pub fn node_hash(&self) -> &NodeHash {
        &self.node
    }

    pub fn view(&self) -> ViewNumber {
        self.view
    }

    pub fn combined_sign(&self) -> &CombinedSign {
        &self.combined_sign
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QCHash([u8; 32]);

impl std::convert::AsRef<[u8]> for QCHash {
    fn as_ref(&self) -> &[u8] {
        return &self.0;
    }
}

impl QCHash {
    fn genesis() -> QCHash {
        QCHash([0; 32])
    }
}
