//! Datastructure for liveness

use hotstuff_rs::{
    data::{GenericQC, ReplicaID, SignKit, TreeNode, ViewNumber},
    msg::Context,
};
use serde::{Deserialize, Serialize};

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
    Ping {
        ctx: Context,
        cont: String,
    },
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
        strategy: BranchSyncStrategy,
        branch: Option<BranchData>,
        status: SyncStatus,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BranchSyncStrategy {
    // Query some specified proposal.
    // Specified { hashes: Vec<NodeHash> },
    Grow {
        grow_from: ViewNumber,
        end: ViewNumber,
        batch_size: usize,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchData {
    pub data: Vec<TreeNode>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncStatus {
    Success,
    ProposalNonExists,
}
