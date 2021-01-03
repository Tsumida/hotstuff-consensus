use serde::{Deserialize, Serialize};

use crate::safety::basic::*;

/// Snapshot for machine's internal state. 
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub view: ViewNumber,
    pub leader: Option<ReplicaID>,
    pub qc_high: Box<GenericQC>, 
    pub leaf: Box<TreeNode>, 
    pub locked_node: Box<TreeNode>, 
    pub last_committed: ViewNumber, 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    pub from: ReplicaID,
    pub view: ViewNumber,
}
