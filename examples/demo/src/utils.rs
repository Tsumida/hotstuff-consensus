use cryptokit::DefaultSignaturer;
use hotstuff_rs::safety::{machine::Machine, voter::Voter};
use hs_data::{ReplicaID, TreeNode, ViewNumber, INIT_NODE, INIT_NODE_HASH, PK};
use hs_network::HotStuffProxy;
use hss::HotstuffStorage;
use pacemaker::{elector::RoundRobinLeaderElector, pacemaker::Pacemaker};
use std::{
    collections::HashMap,
    time::{Duration, SystemTime},
};
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
