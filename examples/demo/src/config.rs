//! Configurations for single node or system

use std::collections::HashMap;

use base64::write;
use hs_data::ReplicaID;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeConfig {
    pub node_name: String,
    pub bootstrap_conf: BootstrapConfig,
    pub self_addr: String,
    pub persistor: PersistentBackend,
    pub server_addr: String,
    pub secret: SecretConfig,
    pub test_config: TestConfig,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum PersistentBackend {
    NoPersistor,
    MySQL { db: String },
}
#[derive(Debug, Serialize, Deserialize)]
pub struct BootstrapConfig {
    pub token: String,
    pub total: usize, 
    pub peer_addrs: Vec<PeerInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PeerInfo {
    pub node_id: String,
    pub addr: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SecretConfig {
    pub sk_id: u64,
    pub sk_share: String,
    pub pk_set: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub enum TestConfig {
    NonTest,
    SingleNode {
        proposal_size: usize,   // in bytes.
        proposal_num: usize,
    },
    LocalCluster {
        // todo!()
    },
}

impl Default for NodeConfig {
    fn default() -> Self {
        NodeConfig {
            node_name: format!("Alice"),
            bootstrap_conf: BootstrapConfig {
                token: format!("test"),
                total: 4, 
                peer_addrs: vec![
                    PeerInfo{
                        node_id: format!("Alice"), addr: format!("127.0.0.1:8880")
                    },
                    PeerInfo{
                        node_id: format!("Bob"), addr: format!("127.0.0.1:8881"),
                    },
                    PeerInfo{
                        node_id: format!("Carol"), addr: format!("127.0.0.1:8882"),
                    },
                    PeerInfo{
                        node_id: format!("Dave"), addr: format!("127.0.0.1:8883")
                    },
                ],
            },
            secret: SecretConfig {
                sk_id: 0,
                sk_share: format!("[5922521929368614765,5036051853562754815,266543411433932454,5480398090728650199]"),
                pk_set: "{\"commit\":{\"coeff\":[[173,216,133,15,41,125,135,255,12,118,193,100,83,228,93,230,191,112,44,6,63,228,181,138,32,191,18,209,248,233,87,31,116,104,77,206,60,14,128,18,83,108,178,213,168,77,202,98],[132,177,74,169,225,50,45,88,147,214,39,140,168,219,228,68,235,86,46,98,124,73,23,55,134,75,114,176,248,217,3,139,22,200,57,5,109,38,16,157,91,201,66,20,104,121,194,229],[182,84,102,12,63,225,153,25,80,53,47,6,84,154,229,99,254,1,164,56,30,55,98,250,200,148,51,2,96,206,43,161,255,124,33,116,72,210,106,41,113,45,65,147,100,113,152,171]]}}".to_string(),
            },
            self_addr: format!("127.0.0.1:8880"),
            persistor: PersistentBackend::MySQL{
                db: format!("mysql://root:helloworld@localhost:3306/hotstuff_test_Alice"), 
            },
            server_addr: format!("127.0.0.1:12340"),
            test_config: TestConfig::SingleNode{
                proposal_num: 20, 
                proposal_size: 1024,   // 1KB
            },
        }
    }
}

#[test]
fn test_config_fmt() {
    let path = "./test-output/alice-config.yml";
    let conf = NodeConfig::default();
    let mut writer = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(path)
        .unwrap();

    serde_yaml::to_writer(&mut writer, &conf).unwrap();
}

#[test]
fn test_read_config() {
    let path = "./test-output/alice-config.yml";

    let reader = std::fs::File::open(path).unwrap();
    let _: NodeConfig = serde_yaml::from_reader(&reader).unwrap();
}
