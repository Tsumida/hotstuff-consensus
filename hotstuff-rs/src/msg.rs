use serde::{Deserialize, Serialize};

use crate::data::{ReplicaID, ViewNumber};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    pub from: ReplicaID,
    pub view: ViewNumber,
}

impl Context{
    #[inline(always)]
    pub fn size(&self) -> usize{
        self.from.len() + std::mem::size_of::<ViewNumber>()
    }
}