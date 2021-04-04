use cryptokit::{DefaultSignaturer, Signaturer};
use hs_data::*;
use pacemaker::data::TimeoutCertificate;
use sqlx::{mysql::MySqlRow, Executor, MySqlPool, Row};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};
use threshold_crypto::serde_impl::SerdeSecret;

use crate::{HotStuffConfig, InMemoryState};

pub(crate) fn map_row_to_proposal(row: MySqlRow) -> (NodeHash, Arc<TreeNode>) {
    let view: ViewNumber = row.get(0);
    let parent_hash: String = row.get(1);
    // let justify_view: ViewNumber = row.get(2);
    // let prop_hash: String = row.get(3);
    let tx: Vec<u8> = row.get(4);
    let node_hash: NodeHash =
        NodeHash::from_vec(&base64::decode(&row.get::<String, _>(6)).unwrap());
    let combined_sign: String = row.get(7);
    let node = Arc::new(TreeNode {
        height: view,
        txs: serde_json::from_slice(&tx).unwrap(),
        parent: NodeHash::from_vec(&base64::decode(&parent_hash).unwrap()),
        justify: GenericQC::new(
            view,
            &node_hash,
            &combined_sign_from_vec_u8(base64::decode(&combined_sign).unwrap()),
        ),
    });

    (node_hash, node)
}

pub(crate) async fn get_one_node_by_view(
    conn_pool: &MySqlPool,
    view: ViewNumber,
) -> Result<(NodeHash, Arc<TreeNode>), sqlx::Error> {
    //  0       1               2           3            4        5           6               7
    // view | parent_hash | justify_view | prop_hash |  tx  | qc.view | qc.node_hash | qc.combined_sign
    sqlx::query(
        "select * from proposal inner join qc 
        on proposal.justify_view=qc.view 
        where proposal.view = ? limit 1",
    )
    .bind(view)
    .map(|row: MySqlRow| map_row_to_proposal(row))
    .fetch_one(conn_pool)
    .await
}

pub(crate) async fn recover_tc_map(
    conn_pool: &MySqlPool,
) -> BTreeMap<ViewNumber, HashSet<TimeoutCertificate>> {
    let mut tc_map = BTreeMap::new();
    sqlx::query("select view, partial_sign from partial_tc;")
        .fetch_all(conn_pool)
        .await
        .unwrap()
        .into_iter()
        .for_each(|row: MySqlRow| {
            let view: ViewNumber = row.get(0);
            let tc: TimeoutCertificate =
                serde_json::from_str(&row.get::<String, usize>(1)).unwrap();
            let tcs = tc_map.entry(view).or_insert(HashSet::new());
            tcs.insert(tc);
        });

    tc_map
}

pub(crate) async fn recover_hotstuff_config(conn_pool: &MySqlPool) -> HotStuffConfig {
    let (token, total, replica_id) =
        sqlx::query("select token, total, self_addr, self_id from hotstuff_conf limit 1; ")
            .map(|row: MySqlRow| {
                let token: String = row.get(0);
                let total: usize = row.get::<u64, _>(1) as usize;
                // let addr: String = row.get(2);
                let replica_id: String = row.get(3);

                (token, total, replica_id)
            })
            .fetch_one(conn_pool)
            .await
            .unwrap();

    let peers_addr: HashMap<ReplicaID, String> = sqlx::query("select replica_id, addr from peers;")
        .map(|row: MySqlRow| {
            let replica_id: String = row.get(0);
            let addr: String = row.get(1);
            (replica_id, addr)
        })
        .fetch_all(conn_pool)
        .await
        .unwrap()
        .into_iter()
        .collect();

    HotStuffConfig {
        token,
        total,
        replica_id,
        peers_addr,
    }
}

pub(crate) async fn recover_hotstuff_state(conn_pool: &MySqlPool) -> InMemoryState {
    // load hotstuff_state
    let (token, current_view, vheight, locked_view, committed_height, executed_view, leaf_view) =
        sqlx::query("select * from hotstuff_state;")
            .map(|row: MySqlRow| {
                //     0          1                 2               3               4                 5            6
                //  | token | current_view  | last_voted_view | locked_view  |  committed_view | executed_view | leaf_view
                let token: String = row.get(0);
                let current_view: ViewNumber = row.get(1);
                let vheight: ViewNumber = row.get(2);
                let locked_view: ViewNumber = row.get(3);
                let committed_view: ViewNumber = row.get(4);
                let executed_view: ViewNumber = row.get(5);
                let leaf_view: ViewNumber = row.get(6);
                (
                    token,
                    current_view,
                    vheight,
                    locked_view,
                    committed_view,
                    executed_view,
                    leaf_view,
                )
            })
            .fetch_one(conn_pool)
            .await
            .unwrap();

    // refactor: stablized INIT_NODE
    let leaf = if leaf_view > 0 {
        get_one_node_by_view(conn_pool, leaf_view).await.unwrap().1
    } else {
        Arc::new(INIT_NODE.clone())
    };

    let b_locked = if locked_view > 0 {
        get_one_node_by_view(conn_pool, locked_view)
            .await
            .unwrap()
            .1
    } else {
        Arc::new(INIT_NODE.clone())
    };

    let last_commit = if executed_view > 0 {
        get_one_node_by_view(conn_pool, executed_view)
            .await
            .unwrap()
            .1
    } else {
        Arc::new(INIT_NODE.clone())
    };

    let qc_high = Arc::new(leaf.justify().clone());

    let state = InMemoryState {
        token,
        leaf,
        qc_high,
        vheight,
        current_view,
        committed_height,
        last_commit,
        b_locked,
    };

    state
}

pub(crate) async fn recover_signaturer(token: &String, conn_pool: &MySqlPool) -> DefaultSignaturer {
    sqlx::query("select pk_set, sk_share, sk_id from crypto where token = ? limit 1; ")
        .bind(token)
        .map(|row: MySqlRow| {
            let pk_set: Vec<u8> = row.get(0);
            let sk_share: Vec<u8> = row.get(1);
            let sk_id: u64 = row.get(2);

            DefaultSignaturer {
                sign_id: sk_id as usize,
                pks: serde_json::from_slice(&pk_set).unwrap(),
                sks: serde_json::from_slice::<SerdeSecret<SK>>(&sk_share)
                    .unwrap()
                    .0,
            }
        })
        .fetch_one(conn_pool)
        .await
        .unwrap()
}

pub(crate) async fn create_tables(conn_pool: &MySqlPool) {
    conn_pool
        .execute(
            "
            CREATE TABLE IF NOT EXISTS `combined_tc`
            (
            `view`           bigint unsigned NOT NULL ,
            `combined_sign`  varchar(512) NOT NULL ,

            PRIMARY KEY (`view`)
            )ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;


            CREATE TABLE IF NOT EXISTS `crypto`
            (
            `token`    varchar(64) NOT NULL ,
            `pk_set`   blob NOT NULL ,
            `sk_share` blob NOT NULL ,
            `sk_id`    bigint unsigned NOT NULL ,

            PRIMARY KEY (`token`)
            )ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;


            CREATE TABLE IF NOT EXISTS `hotstuff_conf`
            (
            `token`     varchar(64) NOT NULL ,
            `total`     bigint unsigned NOT NULL ,
            `self_addr` varchar(64) NOT NULL ,
            `self_id`   varchar(64) NOT NULL ,

            PRIMARY KEY (`token`)
            )ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;


            CREATE TABLE IF NOT EXISTS `hotstuff_state`
            (
            `token`           varchar(64) NOT NULL ,
            `current_view`    bigint unsigned NOT NULL ,
            `last_voted_view` bigint unsigned NOT NULL ,
            `locked_view`     bigint unsigned NOT NULL ,
            `committed_view`  bigint unsigned NOT NULL ,
            `executed_view`   bigint unsigned NOT NULL ,
            `leaf_view`       bigint unsigned NOT NULL ,

            PRIMARY KEY (`token`)
            )ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;


            CREATE TABLE IF NOT EXISTS `partial_tc`
            (
            `view`         bigint unsigned NOT NULL ,
            `partial_sign` varchar(512) NOT NULL ,
            `replica_id`   varchar(64) NOT NULL ,

            PRIMARY KEY (`view`, `replica_id`)
            )ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

            CREATE TABLE IF NOT EXISTS `peers`
            (
            `replica_id` varchar(64) NOT NULL ,
            `addr`       varchar(64) NOT NULL ,

            PRIMARY KEY (`replica_id`)
            )ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;


            CREATE TABLE IF NOT EXISTS `proposal`
            (
            `view`         bigint unsigned NOT NULL ,
            `parent_hash`  varchar(128) NOT NULL ,
            `justify_view` bigint unsigned NOT NULL ,
            `prop_hash`    varchar(128) NOT NULL ,
            `txn`          mediumblob NOT NULL ,

            PRIMARY KEY (`view`)
            )ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

            CREATE TABLE IF NOT EXISTS `qc`
            (
            `view`          bigint unsigned NOT NULL ,
            `node_hash`     varchar(256) NOT NULL ,
            `combined_sign` varchar(512) NOT NULL ,

            PRIMARY KEY (`view`)
            )ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
        ",
        )
        .await
        .unwrap();
}

pub(crate) async fn drop_tables(conn_pool: &MySqlPool) {
    conn_pool
        .execute(
            "
            DROP TABLE IF EXISTS `combined_tc`;
            DROP TABLE IF EXISTS `proposal`;
            DROP TABLE IF EXISTS `qc`;
            DROP TABLE IF EXISTS `partial_tc`;
            DROP TABLE IF EXISTS `hotstuff_state`;
            DROP TABLE IF EXISTS `peers`;
            DROP TABLE IF EXISTS `hotstuff_conf`;
            DROP TABLE IF EXISTS `crypto`;
            ",
        )
        .await
        .unwrap();
}

pub(crate) async fn persist_other_msg(
    conn_pool: &MySqlPool,
    conf: &HotStuffConfig,
    signaturer: &DefaultSignaturer,
) {
    drop_tables(&conn_pool).await;
    create_tables(&conn_pool).await;
    let mut tx = conn_pool.begin().await.unwrap();

    // init peers
    for (id, peers) in &conf.peers_addr {
        sqlx::query("insert into peers (replica_id, addr) values (?, ?); ")
            .bind(id)
            .bind(peers)
            .execute(&mut tx)
            .await
            .unwrap();
    }

    // init conf
    sqlx::query(
        "insert into hotstuff_conf (token, total, self_addr, self_id) values (?, ?, ?, ?);",
    )
    .bind(&conf.token)
    .bind(conf.total as u64)
    .bind(conf.peers_addr.get(&conf.replica_id).unwrap())
    .bind(&conf.replica_id)
    .execute(&mut tx)
    .await
    .unwrap();

    // refactor
    let pk_set = serde_json::to_vec(&signaturer.pks).unwrap();
    let sk_share: SerdeSecret<SK> = SerdeSecret(signaturer.sks.clone()); // panic if type == String
    let encoded_sks = serde_json::to_vec(&sk_share).unwrap();

    // init crypto
    sqlx::query("insert into crypto (token, pk_set, sk_share, sk_id) values (?, ?, ?, ?);")
        .bind(&conf.token)
        .bind(&pk_set)
        .bind(&encoded_sks)
        .bind(signaturer.sign_id() as u64)
        .execute(&mut tx)
        .await
        .unwrap();

    tx.commit().await.unwrap();
}
