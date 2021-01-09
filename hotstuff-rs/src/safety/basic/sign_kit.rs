use super::{Sign, SignID};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SignKit {
    sign: Sign,
    sign_id: SignID,
}

impl SignKit {
    #[inline]
    pub fn sign(&self) -> &Sign {
        &self.sign
    }
    #[inline]
    pub fn sign_id(&self) -> &SignID {
        &self.sign_id
    }
}

impl std::convert::From<(Sign, SignID)> for SignKit {
    fn from(v: (Sign, SignID)) -> Self {
        Self {
            sign: v.0,
            sign_id: v.1,
        }
    }
}
