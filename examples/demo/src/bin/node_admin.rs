//! Load config and start a hotstuff node.
use std::{
    collections::{HashMap, VecDeque},
    io::Read,
    net::SocketAddr,
    process::exit,
    sync::{Arc, Mutex, RwLock},
};

use clap::{App, Arg};
use demo::{
    build_signaturer_from_string, init_hotstuff_node, process_new_tx, NewTxRequest, NewTxResponse,
    ServerSharedState, TX_STATE_INVALID, TX_STATE_PENDING,
};
use hs_data::{TreeNode, Txn};
use hs_network::RpcResp;
use log::{error, info, LevelFilter};
use pacemaker::pacemaker::event_loop_with_functions;
use simplelog::{CombinedLogger, ConfigBuilder, TermLogger, TerminalMode, WriteLogger};
use warp::Filter;

fn main() {
    // Example:
    //  ./node_admin -t=test -i=Alice -s=./test-output/crypto-0 -b=./test-output/peer_addrs -d=mysql://root:helloworld@localhost:3306/hotstuff_test_Alice
    //
    // generate pkset, sks,
    let matches = App::new("hotstuff-admin")
        .version("0.1.0")
        .author("tsuko")
        .about("start hotstuff node")
        .arg(
            Arg::with_name("secret")
                .short("s")
                .long("secret")
                .takes_value(true)
                .help("Load secret file"),
        )
        .arg(
            Arg::with_name("token")
                .short("t")
                .long("token")
                .takes_value(true)
                .help("hotstuff token"),
        )
        .arg(
            Arg::with_name("replica-id")
                .short("i")
                .long("id")
                .takes_value(true)
                .help("ID of hotstuff node."),
        )
        .arg(
            Arg::with_name("bootstrap")
            .short("b")
            .long("bootstrap")
            .takes_value(true)
            .help("Peers addr for bootstrapping."),
        )
        .arg(
            Arg::with_name("db-backend")
                .short("d")
                .long("db-backend")
                .takes_value(true)
                .help("Specify database backend, for example an mysql db connnection. If this is empty, an in-memory stroage will be used and no durability is guaranteed."),
        )
        .arg(
            Arg::with_name("tx-server-addr")
                // .short("d")
                .long("--tx-server-addr")
                .takes_value(true)
                .help("Address for listening client requests."),
        )
        .get_matches();

    let replica_id = matches.value_of("replica-id").unwrap().to_string();

    // recover signaturer
    let mut secret = HashMap::with_capacity(16);
    if let Some(secret_path) = matches.value_of("secret") {
        let secret_path = std::path::Path::new(secret_path);
        if secret_path.is_file() {
            let mut f = std::fs::File::open(secret_path).expect("can't read secret file");
            let mut buf = String::with_capacity(1024);
            f.read_to_string(&mut buf).expect("can't load secret");
            splite_kv_pair(buf, &mut secret);
        } else {
            exit(-1);
        }
    };

    //  format:
    //      <replica_id>=<addr>
    //

    let sk_id: usize = secret
        .get("sk_id")
        .expect("can't find key: sk_id")
        .parse()
        .expect("failed to parse");

    let sk_share = secret.get("sk_share").expect("can't find key: sk_share");
    let pk_set = secret.get("pk_set").expect("can't find key: pk_set");
    let num: usize = secret
        .get("num")
        .expect("can't find key: sk_share")
        .parse()
        .expect("failed to parse");

    let token = matches.value_of("token").expect("need token").to_string();

    let db = matches
        .value_of("db-backend")
        .expect("can't find key: db-backend");

    let mut peer_addrs = HashMap::with_capacity(num);

    let path = matches
        .value_of("bootstrap")
        .expect("need bootstrap config");

    let tx_server_addr: SocketAddr = matches
        .value_of("tx-server-addr")
        .expect("need --tx-server-addr")
        .parse()
        .expect("invalied arg: --tx-server-addr");

    let mut buf = String::with_capacity(1024);
    std::fs::File::open(path)
        .expect("can't open bootstrap config")
        .read_to_string(&mut buf)
        .expect("load failed");

    splite_kv_pair(buf, &mut peer_addrs);

    // --------------------------------------------- check ---------------------------------------------
    if !peer_addrs.contains_key(&replica_id) {
        println!("replica{} not in peer addrs config", &replica_id);
        exit(-1);
    }

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
    ]);

    let signaturer = build_signaturer_from_string(sk_id, sk_share, pk_set);

    let shared_state = Arc::new(RwLock::new(ServerSharedState::default()));

    let s1 = shared_state.clone();
    let shared_state_fn = warp::any().map(move || s1.clone());

    // server for submit new tx.
    let server = warp::post()
        .and(warp::path("new-tx"))
        .and(warp::path::end())
        .and(shared_state_fn.clone())
        .and(warp::body::content_length_limit(2 << 20).and(warp::body::json()))
        .and_then(process_new_tx);

    // Informing new committed proposals
    let inform_fn = |prop: Arc<TreeNode>| {
        let mut sss_unlocked = shared_state.write().unwrap();
        sss_unlocked.commit_queue.push_back(prop.as_ref().clone());
    };

    let observe_fn = || None;

    let fetch = |enable_make_prop: bool| {
        let mut ss = shared_state.write().unwrap();
        if !enable_make_prop || ss.tx_queue.len() == 0 {
            return None;
        }
        let mut txs = Vec::with_capacity(8);
        for _ in 0..8 {
            match ss.tx_queue.pop_front() {
                Some(prop) => txs.push(prop),
                None => break,
            }
        }
        Some(txs)
    };

    info!("init hotstuff node: {}-{}", token, replica_id);

    // --------------------------------------------- run -----------------------------------------------
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async move {
            let pm = init_hotstuff_node(token, num, replica_id, db, peer_addrs, signaturer).await;
            let (_, quit_ch) = tokio::sync::mpsc::channel(1);

            tokio::spawn(warp::serve(server).run(tx_server_addr));

            // pm.run(quit_ch).await.unwrap();
            event_loop_with_functions(pm, quit_ch, observe_fn, inform_fn, fetch)
                .await
                .unwrap();
        });
}

fn splite_kv_pair(buf: String, secret: &mut HashMap<String, String>) {
    buf.lines()
        .map(|line| line.trim().splitn(2, "=").collect::<Vec<&str>>())
        .filter(|kv| kv.len() == 2)
        .for_each(|kv| {
            secret.insert(
                kv.get(0).unwrap().to_string(),
                kv.get(1).unwrap().to_string(),
            );
        });
}
