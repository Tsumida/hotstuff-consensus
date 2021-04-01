//! Demo
//!

pub mod config;
pub mod utils;

use hs_data::{TreeNode, Txn, ViewNumber};
use log::error;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, RwLock},
    time::SystemTime,
};

pub struct TxWrapper {
    pub tx_hash: String,
    pub tx: Txn,
    pub state: &'static str,
}

pub struct ServerSharedState {
    pub tx_queue: VecDeque<Txn>,
    pub tx_pool: HashMap<String, TxWrapper>,
}

impl Default for ServerSharedState {
    fn default() -> Self {
        ServerSharedState {
            tx_queue: VecDeque::with_capacity(16),
            tx_pool: HashMap::with_capacity(16),
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
