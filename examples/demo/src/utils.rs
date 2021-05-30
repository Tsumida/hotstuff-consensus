use cryptokit::DefaultSignaturer;
use hotstuff_rs::safety::{machine::Machine, voter::Voter};
use hs_data::{ReplicaID, TreeNode, ViewNumber, INIT_NODE, INIT_NODE_HASH, INIT_QC, PK};
use hs_network::HotStuffProxy;
use hss::HotstuffStorage;
use pacemaker::{elector::RoundRobinLeaderElector, pacemaker::Pacemaker};
use std::{
    collections::HashMap,
    time::{Duration, SystemTime},
};
use threshold_crypto::{PublicKeySet, SecretKeyShare};

pub async fn init_hotstuff_node(
    token: String,
    total: usize,
    replica_id: ReplicaID,
    db: Option<&str>,
    peer_addrs: HashMap<ReplicaID, String>,
    signaturer: DefaultSignaturer,
) -> Pacemaker<HotstuffStorage> {
    let threshold = (total << 1) / 3;
    let mut elector = RoundRobinLeaderElector::default();
    elector.init(peer_addrs.keys().cloned());

    let voter = Voter::new(threshold, signaturer.clone());

    let (net_adaptor, hs_proxy) =
        hs_network::new_adaptor_and_proxy(replica_id.clone(), peer_addrs.clone()).await;

    let storage: HotstuffStorage = match db {
        Some(db_addr) => {
            hss::init_hotstuff_storage(
                token.clone(),
                total,
                &INIT_NODE,
                &INIT_NODE_HASH,
                replica_id.clone(),
                peer_addrs,
                db_addr,
                signaturer.clone(),
            )
            .await
        }
        None => hss::init_in_mem_storage(
            total,
            &INIT_NODE,
            &INIT_NODE_HASH,
            &INIT_QC,
            replica_id.clone(),
            peer_addrs,
            signaturer.clone(),
        ),
    };

    tokio::spawn(HotStuffProxy::run(hs_proxy));

    let machine = Machine::new(voter, replica_id.clone(), total, None, storage);

    Pacemaker::new(replica_id, elector, machine, net_adaptor, signaturer)
}

pub fn serialize_pks(pks: &PublicKeySet) -> String {
    base64::encode(serde_json::to_vec(pks).unwrap())
}

pub fn deserialize_pks(pks: &str) -> PublicKeySet {
    serde_json::from_slice(&base64::decode(pks).unwrap()).unwrap()
}

pub fn serialize_sks(sks: SecretKeyShare) -> String {
    base64::encode(serde_json::to_vec(&threshold_crypto::serde_impl::SerdeSecret(sks)).unwrap())
}

pub fn deserialize_sks(sks: &str) -> SecretKeyShare {
    serde_json::from_slice::<threshold_crypto::serde_impl::SerdeSecret<SecretKeyShare>>(
        &base64::decode(sks).unwrap(),
    )
    .unwrap()
    .0
}

pub fn build_signaturer_from_string(
    sk_id: usize,
    sk_share: &str,
    pk_set: &str,
) -> DefaultSignaturer {
    // secret is base64 encoded.
    let pkset: PK = deserialize_pks(pk_set);

    let skshare = deserialize_sks(sk_share);

    let view = 19491001u64.to_be_bytes();

    assert!(
        pkset
            .public_key_share(sk_id)
            .verify(&skshare.sign(&view), view),
        "threshold signature verification failed"
    );

    DefaultSignaturer::new(sk_id, pkset, skshare)
}

pub struct HotStuffNodeStat {
    pub committed_prop_list: Vec<ViewNumber>,
    // interval for two adjacent committed proposals.
    pub interval_list: Vec<Duration>,
    pub ts_start: SystemTime,
    pub ts_last_prop: SystemTime,
    pub max_gap: u64,
}

impl Default for HotStuffNodeStat {
    fn default() -> Self {
        HotStuffNodeStat {
            committed_prop_list: Vec::new(),
            interval_list: Vec::new(),
            ts_start: SystemTime::now(),
            ts_last_prop: SystemTime::now(),
            max_gap: 0,
        }
    }
}

impl HotStuffNodeStat {
    pub fn total_time(&self) -> Duration {
        SystemTime::now().duration_since(self.ts_start).unwrap()
    }

    pub fn take_new_committed_prop(&mut self, prop: &TreeNode) {
        let now = SystemTime::now();
        let dur = now.duration_since(self.ts_last_prop).unwrap();

        self.ts_last_prop = now;
        self.max_gap = u64::max(
            self.max_gap,
            self.committed_prop_list
                .last()
                .map_or(0, |view| prop.height() - view),
        );
        self.committed_prop_list.push(prop.height());
        self.interval_list.push(dur);
    }
}
