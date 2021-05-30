//! Generate publickey set and partial secret key.

use std::{io::Write, process::exit};

use clap::{App, Arg};
use demo::{
    config::{self, PeerInfo, PersistentBackend, TestConfig},
    utils::{serialize_pks, serialize_sks},
};
use threshold_crypto::serde_impl::SerdeSecret;

const SC_GEN_CRY: &str = "gen_crypto";
const SC_NODE_CONFIG: &str = "node-config";

const NODE_CONFIG_MEM: &str = "in-memory";
const NODE_CONFIG_NAME: &str = "with-name";

const NODE_NAME_LIST: [&str; 4] = ["Alice", "Bob", "Carol", "Dave"];

fn main() {
    // generate pkset, sks,
    let matches = App::new("threhsold-key generator")
        .version("0.1.0")
        .author("tsuko")
        .about("Testing suite")
        .subcommand(
            App::new(SC_GEN_CRY)
                .about("Generate only threshold key for testing")
                .arg(
                    Arg::with_name("num")
                        .short("n")
                        .long("num")
                        .takes_value(true)
                        .help("The number of hotstuff peers"),
                )
                .arg(
                    Arg::with_name("output")
                        .short("o")
                        .long("output")
                        .takes_value(true)
                        .help("Output keys to specified dir"),
                ),
        )
        .subcommand(
            App::new(SC_NODE_CONFIG)
                .about("Generate node configurations for N nodes")
                .arg(
                    Arg::with_name("num")
                        .short("n")
                        .long("num")
                        .takes_value(true)
                        .help("The number of hotstuff peers"),
                )
                .arg(
                    Arg::with_name("proposal")
                        .short("p")
                        .long("proposal")
                        .takes_value(true)
                        .help(
                            "Specify the number of proposals each hotstuff-consensus node submits",
                        ),
                )
                .arg(
                    Arg::with_name("output")
                        .short("o")
                        .long("output")
                        .takes_value(true)
                        .help("Output keys to specified dir"),
                )
                .arg(
                    Arg::with_name(NODE_CONFIG_NAME)
                        .long("with-name")
                        .takes_value(false)
                        .help("Use name: Alice, Bob, Carol, Dave..."),
                )
                .arg(
                    Arg::with_name(NODE_CONFIG_MEM)
                        .short("m")
                        .long("memory")
                        .takes_value(false)
                        .help("Use in-memory storage (NoPersistor)"),
                ),
        )
        .get_matches();

    if let Some(sub) = matches.subcommand_matches(SC_GEN_CRY) {
        gen_crypto_config(&sub)
    }

    if let Some(sub) = matches.subcommand_matches(SC_NODE_CONFIG) {
        gen_node_config(&sub)
    }
}

fn gen_crypto_config(matches: &clap::ArgMatches) {
    let num = matches
        .value_of("num")
        .expect("expect peer number")
        .parse::<usize>()
        .expect("expect unsigned integer as peer number");

    let (_, pkset, vec_skshare) = hs_data::threshold_sign_kit(num, (num << 1) / 3);
    //  output format:
    //
    //  num=4
    //  threshold=2
    //  sk_id=0
    //  sk_share=ABCDEFG
    //  pk_set=HIJKLMN

    let bs64_pkset = base64::encode(serde_json::to_vec(&pkset).unwrap());

    let conts = vec_skshare.into_iter().map(|(sk_id, skshare)| {
        let cont = [
            format!("num={}", num),
            format!("th={}", (num << 1) / 3),
            format!("sk_id={}", sk_id),
            format!(
                "sk_share={}",
                base64::encode(serde_json::to_vec(&SerdeSecret(skshare)).unwrap()),
            ),
            format!("pk_set={}", bs64_pkset),
        ]
        .join("\n");
        (sk_id, cont)
    });

    if let Some(output_path) = matches.value_of("output") {
        let path = std::path::Path::new(output_path);
        if !path.is_dir() {
            println!("dir not exists");
            exit(-1);
        }

        for (sk_id, cont) in conts {
            match std::fs::File::create(path.join(format!("crypto-{}", sk_id))) {
                Ok(mut f) => {
                    f.write(cont.as_bytes()).unwrap();
                }
                Err(e) => {
                    println!("err:{:?}", e);
                    exit(-1);
                }
            }
        }
    } else {
        for (_, cont) in conts {
            println!("{}\n\n", cont);
        }
    }
}

/// Generate node config (using in-memory storage or shared MySQL).
fn gen_node_config(matches: &clap::ArgMatches) {
    let output = matches.value_of("output");

    let with_name = matches.is_present(NODE_CONFIG_NAME);

    let in_mem = matches.is_present(NODE_CONFIG_MEM);

    let p_num = matches
        .value_of("proposal")
        .map_or(4, |p| p.parse::<usize>().unwrap_or(4));

    let num = matches
        .value_of("num")
        .expect("expect peer number")
        .parse::<usize>()
        .expect("expect unsigned integer as peer number");

    if num > 4 && with_name {
        eprintln!("There should be at most 4 nodes, got {}", num);
        exit(-1);
    }

    let (_, pkset, vec_skshare) = hs_data::threshold_sign_kit(num, (num << 1) / 3);
    let bs64_pkset = serialize_pks(&pkset);

    // gen 4 node config file

    let peer_addrs = (0..num)
        .map(|i| config::PeerInfo {
            node_id: decide_node_id(with_name, i),
            addr: format!("127.0.0.1:{}", 8800 + i),
        })
        .collect::<Vec<PeerInfo>>();

    let configs = peer_addrs
        .iter()
        .zip(vec_skshare)
        .map(|(pif, (sk_id, sks))| {
            let mut node_conf = config::NodeConfig::default();
            node_conf.node_name = pif.node_id.clone();
            node_conf.bootstrap_conf.peer_addrs = peer_addrs.clone();

            node_conf.persistor = if !in_mem {
                PersistentBackend::MySQL {
                    db: format!(
                        "mysql://root:helloworld@127.0.0.1:3306/hotstuff_{}_{}",
                        &node_conf.bootstrap_conf.token, &node_conf.node_name
                    ),
                }
            } else {
                PersistentBackend::NoPersistor
            };

            node_conf.server_addr = format!("127.0.0.1:{}", 12300 + sk_id);

            node_conf.secret.sk_id = sk_id as u64;
            node_conf.secret.sk_share = serialize_sks(sks);
            node_conf.secret.pk_set = bs64_pkset.clone();
            if let TestConfig::SingleNode {
                ref mut proposal_num,
                ref mut proposal_size,
            } = node_conf.test_config
            {
                *proposal_num = p_num;
                *proposal_size = 4096; // 4KB
            }

            node_conf
        });

    if let Some(output_dir) = output {
        // -> files
        let path = std::path::Path::new(output_dir);
        if !path.is_dir() {
            eprintln!("expect dir");
            exit(-1);
        }

        configs.into_iter().enumerate().for_each(|(_, config)| {
            match std::fs::File::create(
                path.join(format!("{}-config.yml", &config.node_name.to_lowercase())),
            ) {
                Ok(f) => {
                    serde_yaml::to_writer(f, &config).unwrap();
                }
                Err(e) => {
                    println!("failed to create files. got err: {:?}", e);
                    exit(-1);
                }
            }
        });
    } else {
        // -> stdout
        let mut to = std::io::stdout();
        configs.into_iter().for_each(|config| {
            serde_yaml::to_writer(&mut to, &config).unwrap();
            to.write(b"\n\n").unwrap();
        });
    }
}

/// Decide name of node.
fn decide_node_id(with_name: bool, id: usize) -> String {
    if !with_name {
        format!("node-{}", id)
    } else {
        NODE_NAME_LIST[id].to_string()
    }
}
