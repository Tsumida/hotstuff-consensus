//! Utils.

use threshold_crypto::{PublicKeySet, SecretKeySet, SecretKeyShare};

pub fn threshold_sign_kit(
    n: usize,
    t: usize,
) -> (SecretKeySet, PublicKeySet, Vec<(usize, SecretKeyShare)>) {
    assert!(t <= n);

    let s = SecretKeySet::random(t, &mut rand::thread_rng());

    let vec_sk = (0..n).map(|i| (i, s.secret_key_share(i))).collect();
    let pks = s.public_keys();
    (s, pks, vec_sk)
}
