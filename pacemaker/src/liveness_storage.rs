//! Storage for liveness evnet, message.
//!

use hotstuff_rs::data::{GenericQC, ViewNumber};

use crate::data::TimeoutCertificate;

pub enum LivenessStorageErr {}
pub trait LivenessStorage {
    /// Idempotently appending new TimeoutCertificates.
    /// Return `true` if there are at least `n-f` TCs from different replicas.
    fn append_tc(&mut self, tc: &TimeoutCertificate) -> bool;

    /// Return `true` if there are at least `n-f` TCs from different replicas.
    fn is_reach_threshold(&mut self, view: ViewNumber) -> bool;

    /// Update QC-High (highest GenericQC this replica has ever seen)
    fn update_qc_high(&mut self, qc: &GenericQC);
}
