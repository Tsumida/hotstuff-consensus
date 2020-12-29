use serde::{Deserialize, Serialize};

use crate::safety::basic::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub view: ViewNumber,
    pub leader: Option<ReplicaID>,
    pub threshold: usize,
    pub leaf: String,
    pub qc_high: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    pub from: ReplicaID,
    pub view: ViewNumber,
}

pub struct StorageState {
    pub view: ViewNumber,
    pub commit_height: ViewNumber,
    pub executed_height: ViewNumber,
    pub leaf_height: ViewNumber,
}
