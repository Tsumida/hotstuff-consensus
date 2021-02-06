//! Datastructure for liveness

use hotstuff_rs::{
    data::{GenericQC, NodeHash, ReplicaID, SignKit, TreeNode, ViewNumber},
    msg::Context,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub type PeerID = String;

/// TimeoutCertificate(TC) is a signature for a certain view.
/// A replica asserts view with viewNumber=v is past once it received n-f different TCs from other replicas.
/// Inspired by [LibraBFT](https://developers.diem.com/papers/diem-consensus-state-machine-replication-in-the-diem-blockchain/2020-05-26.pdf).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutCertificate {
    from: ReplicaID,
    view: ViewNumber,
    view_sign: SignKit, // partial sign
    qc_high: GenericQC,
}

impl TimeoutCertificate {
    pub fn new(from: ReplicaID, view: ViewNumber, view_sign: SignKit, qc_high: GenericQC) -> Self {
        Self {
            from,
            view,
            view_sign,
            qc_high,
        }
    }

    pub fn from(&self) -> &ReplicaID {
        &self.from
    }

    pub fn view(&self) -> ViewNumber {
        self.view
    }

    pub fn view_sign(&self) -> &SignKit {
        &self.view_sign
    }

    pub fn qc_high(&self) -> &GenericQC {
        &self.qc_high
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PeerEvent {
    // Proposal from leader. May be stale.
    NewProposal {
        ctx: Context,
        prop: Box<TreeNode>,
    },
    AcceptProposal {
        ctx: Context,
        prop: Box<TreeNode>,
        sign: Option<Box<SignKit>>,
    },
    // Timeout message from other replica.
    Timeout {
        ctx: Context,
        tc: TimeoutCertificate,
    },
    BranchSyncRequest {
        ctx: Context,
        strategy: BranchSyncStrategy,
    },
    BranchSyncResponse {
        ctx: Context,
        branch: Vec<TreeNode>,
    },
}

impl PeerEvent {
    // todo
    pub fn to_be_bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();

        v
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BranchSyncStrategy {
    // Query some specified proposal.
    Specified { hashes: Vec<NodeHash> },
}
