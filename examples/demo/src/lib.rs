//! Demo
//!

use cryptokit::DefaultSignaturer;
use hotstuff_rs::safety::{machine::Machine, voter::Voter};
use hs_data::{combined_sign_from_vec_u8, ReplicaID, INIT_NODE, INIT_NODE_HASH, PK};
use hs_network::HotStuffProxy;
use hss::HotstuffStorage;
use pacemaker::{elector::RoundRobinLeaderElector, pacemaker::Pacemaker};
use std::collections::HashMap;
use threshold_crypto::SecretKeyShare;

pub async fn init_hotstuff_node(
    token: String,
    total: usize,
    replica_id: ReplicaID,
    db: &str,
    peer_addrs: HashMap<ReplicaID, String>,
    signaturer: DefaultSignaturer,
) -> Pacemaker<HotstuffStorage> {
    let threshold = (total << 1) / 3;
    let mut elector = RoundRobinLeaderElector::default();
    elector.init(peer_addrs.keys().cloned());

    let voter = Voter::new(threshold, signaturer.clone());

    let (net_adaptor, hs_proxy) =
        hs_network::new_adaptor_and_proxy(replica_id.clone(), peer_addrs.clone()).await;

    let storage: HotstuffStorage = hss::init_hotstuff_storage(
        token.clone(),
        total,
        &INIT_NODE,
        &INIT_NODE_HASH,
        replica_id.clone(),
        peer_addrs,
        db,
        signaturer.clone(),
    )
    .await;

    tokio::spawn(HotStuffProxy::run(hs_proxy));

    let machine = Machine::new(voter, replica_id.clone(), total, None, storage);

    Pacemaker::new(replica_id, elector, machine, net_adaptor, signaturer)
}

pub fn build_signaturer_from_string(
    sk_id: usize,
    sk_share: &str,
    pk_set: &str,
) -> DefaultSignaturer {
    let pkset: PK = serde_json::from_str(pk_set).unwrap();

    let serde_secret: threshold_crypto::serde_impl::SerdeSecret<SecretKeyShare> =
        serde_json::from_str(sk_share).unwrap();

    let skshare = serde_secret.0;

    let view = 2021u64.to_be_bytes();

    assert!(pkset
        .public_key_share(sk_id)
        .verify(&skshare.sign(&view), view));

    DefaultSignaturer::new(sk_id, pkset, skshare)
}
