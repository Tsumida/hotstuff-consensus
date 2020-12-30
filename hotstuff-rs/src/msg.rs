use serde::{Deserialize, Serialize};

use crate::safety::basic::*;

/// Snapshot for machine's internal state. 
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub view: ViewNumber,
    pub leader: Option<ReplicaID>,
    // base64 code for leaf
    pub leaf: String,
    // base64 code for qcHigh
    pub qc_high: String,
    pub locked_node_height: ViewNumber, 
    pub last_committed_height: ViewNumber, 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    pub from: ReplicaID,
    pub view: ViewNumber,
}
