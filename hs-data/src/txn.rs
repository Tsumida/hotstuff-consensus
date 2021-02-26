use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Txn(pub Vec<u8>);

impl AsRef<[u8]> for Txn {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Txn {
    pub fn new(cont: impl AsRef<[u8]>) -> Self {
        Txn(cont.as_ref().to_vec())
    }
}
