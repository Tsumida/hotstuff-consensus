//! Load config and start a hotstuff node.
use std::{collections::HashMap, io::Read, process::exit};

use clap::{App, Arg};
use demo::{build_signaturer_from_string, init_hotstuff_node};
use log::{info, LevelFilter};
use simplelog::{CombinedLogger, Config, ConfigBuilder, TermLogger, TerminalMode, WriteLogger};

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
        .get_matches();

    //
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

    let mut buf = String::with_capacity(1024);
    std::fs::File::open(path)
        .expect("can't open bootstrap config")
        .read_to_string(&mut buf)
        .expect("load failed");

    splite_kv_pair(buf, &mut peer_addrs);

    // --------------------------------------------- check ---------------------------------------------
    if !peer_addrs.contains_key(&replica_id) {
        println!("replica-id {} not in peer addrs config", &replica_id);
        exit(-1);
    }

    if peer_addrs.len() != num {
        println!(
            "imcompatible number of peers {} while total = {}",
            peer_addrs.len(),
            num
        );
    }

    let _ = CombinedLogger::init(vec![
        TermLogger::new(
            LevelFilter::Debug,
            ConfigBuilder::new()
                .add_filter_ignore(format!("sqlx"))
                .add_filter_ignore(format!("rustls"))
                .build(),
            TerminalMode::Mixed,
        ),
        WriteLogger::new(
            LevelFilter::Debug,
            ConfigBuilder::new()
                .add_filter_ignore(format!("sqlx"))
                .add_filter_ignore(format!("rustls"))
                .build(),
            std::fs::File::create(format!("./hotstuff-{}-{}.log", token, replica_id)).unwrap(),
        ),
    ]);

    // --------------------------------------------- init ----------------------------------------------
    let signaturer = build_signaturer_from_string(sk_id, sk_share, pk_set);

    info!("init hotstuff node: {}-{}", token, replica_id);
    // start hotstuff
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async move {
            let mut pm =
                init_hotstuff_node(token, num, replica_id, db, peer_addrs, signaturer).await;
            let (_, quit_ch) = tokio::sync::mpsc::channel(1);

            pm.run(quit_ch).await.unwrap();
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
