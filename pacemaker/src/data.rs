//! Datastructure for liveness

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use cryptokit::{DefaultSignaturer, Signaturer};
use hs_data::msg::Context;
use hs_data::{CombinedSign, GenericQC, ReplicaID, SignKit, TreeNode, ViewNumber};
use serde::{Deserialize, Serialize};

pub type PeerID = String;

/// TimeoutCertificate(TC) is a signature for a certain view.
/// A replica asserts view with viewNumber=v is past once it received n-f different TCs from other replicas.
/// Inspired by [LibraBFT](https://developers.diem.com/papers/diem-consensus-state-machine-replication-in-the-diem-blockchain/2020-05-26.pdf).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutCertificate {
    from: ReplicaID,
    view: ViewNumber,
    view_sign: SignType,
    qc_high: GenericQC,
}

impl PartialEq for TimeoutCertificate {
    fn eq(&self, other: &Self) -> bool {
        self.view == other.view && self.from == other.from
    }
}

impl Eq for TimeoutCertificate {}

impl Hash for TimeoutCertificate {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // view_sign is verified by Signaturear.
        self.view.hash(state);
        self.from.hash(state);
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SignType {
    Partial(SignKit),
    Combined(CombinedSign),
}

impl TimeoutCertificate {
    /// Create partial tc.
    pub fn new(from: ReplicaID, view: ViewNumber, view_sign: SignKit, qc_high: GenericQC) -> Self {
        Self {
            from,
            view,
            view_sign: SignType::Partial(view_sign),
            qc_high,
        }
    }

    pub fn compressed(
        from: ReplicaID,
        view: ViewNumber,
        view_sign: CombinedSign,
        qc_high: GenericQC,
    ) -> Self {
        Self {
            from,
            view,
            view_sign: SignType::Combined(view_sign),
            qc_high,
        }
    }

    pub fn from(&self) -> &ReplicaID {
        &self.from
    }

    pub fn view(&self) -> ViewNumber {
        self.view
    }

    pub fn view_sign(&self) -> &SignType {
        &self.view_sign
    }

    pub fn qc_high(&self) -> &GenericQC {
        &self.qc_high
    }
}

/// Combine TCs with paritial signature into a new TC with threshold signature.
/// Return `None` while it cannot produce a threshold signature.
/// This function may help compressing time certificates,
/// Make sure the threshold signature algorithm will **produce the same output for any different partial signature set ps1 and ps2**,
/// where `|ps1| > t, |ps2| > t` and `t` is the threshold, since different node set may receive different time certificate set.
pub fn combine_time_certificates<'a>(
    signaturer: &DefaultSignaturer,
    from: &ReplicaID,
    view: ViewNumber,
    qc_high: GenericQC,
    tcs: impl IntoIterator<Item = &'a TimeoutCertificate>,
) -> Option<TimeoutCertificate> {
    let iter_partial_signs = tcs
        .into_iter()
        .filter(|tc| {
            if let SignType::Partial(_) = tc.view_sign {
                true
            } else {
                false
            }
        })
        .map(|tc| {
            if let SignType::Partial(ps) = tc.view_sign() {
                ps
            } else {
                unreachable!()
            }
        });

    match signaturer.combine_partial_sign(iter_partial_signs) {
        Ok(s) => Some(TimeoutCertificate::compressed(
            String::default(),
            view,
            *s,
            qc_high,
        )),
        Err(_) => None,
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
    NewView {
        ctx: Context,
        qc_high: Box<GenericQC>,
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
