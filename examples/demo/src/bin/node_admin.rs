//! Load config and start a hotstuff node.
use std::{
    collections::HashMap,
    io::Read,
    net::SocketAddr,
    sync::{Arc, RwLock},
};

use clap::{App, Arg};
use demo::{
    build_signaturer_from_string, config::NodeConfig, init_hotstuff_node, process_new_tx, query_tx,
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
        /*
        TermLogger::new(
            LevelFilter::Debug,
            ConfigBuilder::new()
                .add_filter_ignore(format!("sqlx"))
                .add_filter_ignore(format!("rustls"))
                .build(),
            TerminalMode::Mixed,
        ),*/
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
    let shared_state_fn = warp::any().map(move || s1.clone());

    // server for submit new tx.
    let new_tx_server = warp::post()
        .and(warp::path("new-tx"))
        .and(warp::path::end())
        .and(shared_state_fn.clone())
        .and(warp::body::content_length_limit(2 << 20).and(warp::body::json()))
        .and_then(process_new_tx);

    // /tx/:string
    let tx_query = warp::get()
        .and(warp::path("tx"))
        .and(shared_state_fn.clone())
        .and(warp::path::param())
        .and(warp::path::end())
        .and_then(query_tx);

    // define
    let inform_fn = |prop: Arc<TreeNode>| {
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

        let last_committed = prop.height();
        sss_unlocked.committed_list.push(last_committed);
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
        let sss_unlocked = shared_state.write().unwrap();
        // println!("committed proposal: {:?}", sss_unlocked.committed_list);
        info!("call finilizer: \n");
        for ck in sss_unlocked.committed_list.chunks(8) {
            println!("committed proposal: {:?}", ck);
        }
    };

    // tell node to stop.
    let end_fn = |cur_view: ViewNumber| cur_view > (block_num * num) as u64;

    info!("init hotstuff node: {}-{}", token, replica_id);

    // --------------------------------------------- run -----------------------------------------------
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async move {
            let pm = init_hotstuff_node(token, num, replica_id, &db, peer_addrs, signaturer).await;
            let (_, quit_ch) = tokio::sync::mpsc::channel(1);

            tokio::spawn(warp::serve(new_tx_server.or(tx_query)).run(tx_server_addr));

            // pm.run(quit_ch).await.unwrap();
            event_loop_with_functions(pm, quit_ch, observe_fn, inform_fn, fetch, end_fn, finilizer)
                .await
                .unwrap();
        });
}
