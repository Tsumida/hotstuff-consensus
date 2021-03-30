//! Demo
//!

pub mod config;

use cryptokit::DefaultSignaturer;
use hotstuff_rs::safety::{machine::Machine, voter::Voter};
use hs_data::{ReplicaID, Txn, ViewNumber, INIT_NODE, INIT_NODE_HASH, PK};
use hs_network::HotStuffProxy;
use hss::HotstuffStorage;
use log::error;
use pacemaker::{elector::RoundRobinLeaderElector, pacemaker::Pacemaker};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, RwLock},
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

pub struct TxWrapper {
    pub tx_hash: String,
    pub tx: Txn,
    pub state: &'static str,
}

pub struct ServerSharedState {
    pub tx_queue: VecDeque<Txn>,
    pub tx_pool: HashMap<String, TxWrapper>,
    pub committed_list: Vec<ViewNumber>,
}

impl Default for ServerSharedState {
    fn default() -> Self {
        ServerSharedState {
            tx_queue: VecDeque::with_capacity(16),
            tx_pool: HashMap::with_capacity(16),
            committed_list: Vec::new(),
        }
    }
}

pub const TX_STATE_PENDING: &str = "pending";
pub const TX_STATE_QC_FORMED: &str = "qc formed";
pub const TX_STATE_LOCKED: &str = "locked";
pub const TX_STATE_COMMITTED: &str = "committed";
pub const TX_STATE_INVALID: &str = "invalid";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewTxRequest {
    pub tx_hash: String,

    /// base64 encoded bytes.
    pub tx: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewTxResponse {
    pub tx_hash: String,
    pub state: &'static str,
}

pub async fn process_new_tx(
    sss: Arc<RwLock<ServerSharedState>>,
    request: NewTxRequest,
) -> Result<impl warp::Reply, warp::Rejection> {
    let mut resp = NewTxResponse {
        tx_hash: request.tx_hash,
        state: TX_STATE_INVALID,
    };

    match base64::decode(request.tx) {
        Ok(buf) => {
            let txn = Txn(resp.tx_hash.as_bytes().to_vec());
            {
                let mut sss_unlocked = sss.write().unwrap();
                sss_unlocked.tx_pool.insert(
                    resp.tx_hash.clone(),
                    TxWrapper {
                        tx_hash: resp.tx_hash.clone(),
                        tx: txn.clone(),
                        state: TX_STATE_PENDING,
                    },
                );
                sss_unlocked.tx_queue.push_back(txn);
            }
            resp.state = TX_STATE_PENDING;
        }
        Err(e) => {
            error!("base64 decode failed");
        }
    }

    Ok(warp::reply::json(&resp))
}

pub async fn query_tx(
    sss: Arc<RwLock<ServerSharedState>>,
    tx_hash: String,
) -> Result<impl warp::Reply, warp::Rejection> {
    let mut resp = NewTxResponse {
        tx_hash,
        state: TX_STATE_INVALID,
    };

    if let Some(txw) = sss.read().unwrap().tx_pool.get(&resp.tx_hash) {
        resp.state = txw.state;
    }

    Ok(warp::reply::json(&resp))
}
