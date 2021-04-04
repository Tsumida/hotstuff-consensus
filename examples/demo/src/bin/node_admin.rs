//! Load config and start a hotstuff node.
use std::{
    collections::HashMap,
    io::Read,
    net::SocketAddr,
    sync::{Arc, Mutex, RwLock},
    time::SystemTime,
};

use clap::{App, Arg};
use demo::{
    config::{NodeConfig, TestConfig},
    process_new_tx, query_tx,
    utils::{build_signaturer_from_string, init_hotstuff_node, HotStuffNodeStat},
    ServerSharedState, TX_STATE_COMMITTED,
};
use hs_data::{ReplicaID, TreeNode, Txn, ViewNumber};
use log::{info, LevelFilter};
use pacemaker::pacemaker::event_loop_with_functions;
use simplelog::{CombinedLogger, ConfigBuilder, WriteLogger};
use warp::Filter;

fn main() {
    // generate pkset, sks,
    let matches = App::new("hotstuff-admin")
        .version("0.1.0")
        .author("tsuko")
        .about("Hotstuff node administration")
        .arg(
            Arg::with_name("config-file")
                .short("c")
                .long("config")
                .takes_value(true)
                .help("Load config file"),
        )
        .get_matches();

    // load config
    let config_path = matches.value_of("config-file").expect("Need config file");
    let mut conf_str = String::new();
    let _ = std::fs::File::open(config_path)
        .unwrap()
        .read_to_string(&mut conf_str)
        .unwrap();
    let config: NodeConfig = serde_yaml::from_str(&conf_str).expect("can't parse config file");

    // recover signaturer
    let replica_id = config.node_name;
    let sk_id: usize = config.secret.sk_id as usize;
    let sk_share = config.secret.sk_share;
    let pk_set = config.secret.pk_set;
    let num: usize = config.bootstrap_conf.total;
    let token = config.bootstrap_conf.token;
    let (block_size, block_num) = match config.test_config {
        demo::config::TestConfig::SingleNode {
            proposal_num,
            proposal_size,
        } => (proposal_size, proposal_num),
        _ => panic!("unsupported"),
    };

    let flush_interval = config.pm_conf.flush_interval;
    let non_leader_wait = config.pm_conf.non_leader_wait;

    let is_test = if let TestConfig::SingleNode { .. } = config.test_config {
        true
    } else {
        false
    };

    // update static var
    pacemaker::pacemaker::FLUSH_INTERVAL.get_or_init(|| u64::max(100, flush_interval));
    pacemaker::pacemaker::DUR_REPLICA_WAIT.get_or_init(|| u64::max(100, non_leader_wait));

    let db = match config.persistor {
        demo::config::PersistentBackend::MySQL { db } => db,
        _ => panic!("unsupported type"),
    };

    let peer_addrs: HashMap<ReplicaID, String> = config
        .bootstrap_conf
        .peer_addrs
        .into_iter()
        .map(|pi| (pi.node_id, pi.addr))
        .collect();

    let tx_server_addr: SocketAddr = config
        .server_addr
        .parse()
        .expect("can't parse tx-server addr");

    // --------------------------------------------- check ---------------------------------------------
    if peer_addrs.len() != num {
        println!(
            "imcompatible number of peers {} while total = {}",
            peer_addrs.len(),
            num
        );
    }

    // --------------------------------------------- init ----------------------------------------------
    let _ = CombinedLogger::init(vec![
        WriteLogger::new(
            LevelFilter::Debug,
            ConfigBuilder::new()
                .add_filter_ignore(format!("sqlx"))
                .add_filter_ignore(format!("rustls"))
                .build(),
            std::fs::File::create(format!("./hotstuff-{}-{}.log", token, replica_id)).unwrap(),
        ),
        WriteLogger::new(
            LevelFilter::Debug,
            ConfigBuilder::new()
                .add_filter_allow(format!("hotstuff_rs"))
                .build(),
            std::fs::File::create(format!("./hotstuff-{}-{}-machine.log", token, replica_id))
                .unwrap(),
        ),
    ]);

    let signaturer = build_signaturer_from_string(sk_id, &sk_share, &pk_set);
    let shared_state = Arc::new(RwLock::new(ServerSharedState::default()));
    let s1 = shared_state.clone();
    let stat = Arc::new(Mutex::new(HotStuffNodeStat::default()));
    // --------------------------------------------- tx-server ----------------------------------------------
    let shared_state_fn = warp::any().map(move || s1.clone());

    // URL: /new-tx
    // Fn : Summit new transaction.
    let tx_summit = warp::post()
        .and(warp::path("new-tx"))
        .and(warp::path::end())
        .and(shared_state_fn.clone())
        .and(warp::body::content_length_limit(2 << 20).and(warp::body::json()))
        .and_then(process_new_tx);

    // URL: /tx/<tx-id>
    // Fn : Query TX status.
    let tx_query = warp::get()
        .and(warp::path("tx"))
        .and(shared_state_fn.clone())
        .and(warp::path::param())
        .and(warp::path::end())
        .and_then(query_tx);

    let tx_server = tx_summit.or(tx_query);

    // --------------------------------------------- hooks ----------------------------------------------
    // inform_fn should not keep prop in case of memery leak.
    let inform_fn = |prop: Arc<TreeNode>| {
        {
            let mut sss_unlocked = shared_state.write().unwrap();
            // update txn.
            for proposal in prop.tx() {
                // interpret
                sss_unlocked
                    .tx_pool
                    .entry(String::from_utf8(proposal.0.clone()).unwrap())
                    .and_modify(|txw| {
                        txw.state = TX_STATE_COMMITTED;
                    });
            }
        }

        stat.lock().unwrap().take_new_committed_prop(&prop);
    };

    // update imformation
    let observe_fn = || None;

    let fetch = |enable_make_prop: bool| {
        if !enable_make_prop {
            return None;
        }
        Some(vec![Txn(vec![0u8; block_size])])
    };

    let finilizer = || {
        if is_test {
            info!("finilizing");
            let unlocked_stat = stat.lock().unwrap();
            info!("committed:  {:?}", unlocked_stat.committed_prop_list);
            info!(
                "interval(ms):  {:?}",
                unlocked_stat
                    .interval_list
                    .iter()
                    .map(|i| i.as_millis())
                    .collect::<Vec<u128>>()
            );
            info!("total: {} ms", unlocked_stat.total_time().as_millis());
        }
    };

    let end_fn = |cur_view: ViewNumber| cur_view > (block_num * num) as u64;

    // --------------------------------------------- run -----------------------------------------------
    info!("init hotstuff node: {}-{}", token, &replica_id);

    {
        stat.lock().unwrap().ts_start = SystemTime::now();
    }

    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async move {
            let pm = init_hotstuff_node(token, num, replica_id, &db, peer_addrs, signaturer).await;
            let (_, quit_ch) = tokio::sync::mpsc::channel(1);

            tokio::spawn(warp::serve(tx_server).run(tx_server_addr));

            event_loop_with_functions(pm, quit_ch, observe_fn, inform_fn, fetch, end_fn, finilizer)
                .await
                .unwrap();
        });
}
