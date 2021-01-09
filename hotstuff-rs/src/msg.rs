use serde::{Deserialize, Serialize};

use crate::safety::basic::{GenericQC, ReplicaID, TreeNode, ViewNumber};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    pub from: ReplicaID,
    pub view: ViewNumber,
}
