use super::{Sign, SignID};
use serde::{Deserialize, Serialize};
use threshold_crypto::SecretKeySet;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SignKit {
    sign: Sign,
    sign_id: SignID,
}

impl SignKit {
    #[inline(always)]
    pub fn new(sign_id: usize, sign: Sign) -> Self {
        Self { sign_id, sign }
    }

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

pub fn threshold_sign_kit(
    n: usize,
    t: usize,
) -> (SecretKeySet, crate::PK, Vec<(usize, crate::SK)>) {
    assert!(t <= n);

    let s = threshold_crypto::SecretKeySet::random(t, &mut rand::thread_rng());

    let vec_sk = (0..n).map(|i| (i, s.secret_key_share(i))).collect();
    let pks = s.public_keys();
    (s, pks, vec_sk)
}
