//! test for HotstuffStorage

use cryptokit::DefaultSignaturer;
use hs_data::{combined_sign_from_vec_u8, threshold_sign_kit, CombinedSign, NodeHash};
use hss::HotstuffStorage;
use sqlx::{mysql::MySqlRow, Executor, Row};
use std::{future::Future, mem::MaybeUninit};

fn init_logger() {
    use simplelog::*;
    let _ = CombinedLogger::init(vec![TermLogger::new(
        LevelFilter::Info,
        Config::default(),
        TerminalMode::Mixed,
    )]);
}

fn default_test_kit() -> impl Future<Output = HotstuffStorage> {
    let total = 4;
    let (sks, pks, _) = threshold_sign_kit(total, 2);
    let id = 1;
    let self_id = format!("replica-{}", id);
    let peers_addr = vec![
        (format!("replica-0"), format!("localhost:8000")),
        (format!("replica-1"), format!("localhost:8001")),
        (format!("replica-2"), format!("localhost:8002")),
        (format!("replica-3"), format!("localhost:8003")),
    ]
    .into_iter()
    .collect();

    let mysql_addr = "mysql://root:helloworld@localhost:3306/hotstuff_test_mocker";
    let signaturer = DefaultSignaturer::new(id, pks.clone(), sks.secret_key_share(id));
    hss::init_hotstuff_storage(total, self_id, peers_addr, mysql_addr, signaturer)
}

struct MockerDB {
    conn: sqlx::mysql::MySqlPool,
}

impl MockerDB {
    async fn new() -> Self {
        let conn = sqlx::mysql::MySqlPool::connect(
            "mysql://root:helloworld@localhost:3306/hotstuff_test_mocker",
        )
        .await
        .unwrap();

        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS `combined_tc`
            (
             `view`          bigint unsigned NOT NULL ,
             `combined_sign` varchar(128) NOT NULL ,
            
            PRIMARY KEY (`view`)
            );
            
            
            CREATE TABLE IF NOT EXISTS `hotstuff_state`
            (
             `token`           varchar(64) NOT NULL ,
             `current_view`    bigint unsigned NOT NULL ,
             `last_voted_view` bigint unsigned NOT NULL ,
             `locked_view`     bigint unsigned NOT NULL ,
             `committed_view`  bigint unsigned NOT NULL ,
             `executed_view`   bigint unsigned NOT NULL ,
             `leaf_view`       bigint NOT NULL ,
            
            PRIMARY KEY (`token`)
            );
            
            
            CREATE TABLE IF NOT EXISTS `partial_tc`
            (
             `view`         bigint unsigned NOT NULL ,
             `partial_sign` varchar(128) NOT NULL ,
             `replica_id`   varchar(64) NOT NULL ,
            
            PRIMARY KEY (`view`, `replica_id`)
            );
            
            CREATE TABLE IF NOT EXISTS `peers`
            (
             `replica_id` varchar(64) NOT NULL ,
             `addr`       varchar(64) NOT NULL ,
            
            PRIMARY KEY (`replica_id`)
            );
            
            
            CREATE TABLE IF NOT EXISTS `hotstuff_conf`
            (
             `token`     varchar(64) NOT NULL ,
             `total`     bigint unsigned NOT NULL ,
             `self_id`   varchar(64) NOT NULL ,
             `self_addr` varchar(64) NOT NULL ,
            
            PRIMARY KEY (`token`)
            );
            
            
            CREATE TABLE IF NOT EXISTS `proposal`
            (
             `view`         bigint unsigned NOT NULL ,
             `parent_hash`  varchar(128) NOT NULL ,
             `justify_view` bigint unsigned NOT NULL ,
             `prop_hash`    varchar(128) NOT NULL , # 这个proposal本身的hash
             `txn`          mediumblob NOT NULL ,
            
            PRIMARY KEY (`view`)
            );
            
            CREATE TABLE IF NOT EXISTS `qc`
            (
             `view`          bigint unsigned NOT NULL ,
             `node_hash`     varchar(128) NOT NULL , # qc指向的node的哈希
             `combined_sign` varchar(128) NOT NULL ,
            
            PRIMARY KEY (`view`)
            );
            ",
        )
        .await
        .unwrap();

        MockerDB { conn }
    }

    async fn close(&mut self) {
        self.conn
            .execute(
                "
            DROP TABLE IF EXISTS `combined_tc`;
            DROP TABLE IF EXISTS `proposal`;
            DROP TABLE IF EXISTS `qc`;
            DROP TABLE IF EXISTS `partial_tc`;
            DROP TABLE IF EXISTS `hotstuff_state`;
            DROP TABLE IF EXISTS `peers`;
            DROP TABLE IF EXISTS `hotstuff_conf`;
            ",
            )
            .await
            .unwrap();
    }
}

/// Test and usage of sqlx.
async fn test_1() -> Result<(), sqlx::Error> {
    let mut db = MockerDB::new().await;
    let init_prop = hs_data::INIT_NODE.clone();
    let parent_hash: String =
        base64::encode::<NodeHash>(unsafe { MaybeUninit::uninit().assume_init() });
    let node_hash = base64::encode(hs_data::INIT_NODE_HASH.as_ref());
    let combined_sign: CombinedSign = unsafe { MaybeUninit::uninit().assume_init() };
    let sign = base64::encode(&combined_sign.to_bytes());

    // stablization
    let mut tx = db.conn.begin().await.unwrap();

    sqlx::query(
        "
        insert into qc
        (view, node_hash, combined_sign)
        values
        (?, ?, ?)
    ;",
    )
    .bind(0u64)
    .bind(&node_hash)
    .bind(&sign)
    .execute(&mut tx)
    .await
    .unwrap();

    sqlx::query(
        "
        insert into proposal
        (view, parent_hash, justify_view, prop_hash, txn)
        values
        (?, ?, ?, ?, ?)
    ;",
    )
    .bind(&init_prop.height())
    .bind(&parent_hash)
    .bind(0u64)
    .bind(&node_hash)
    .bind(&serde_json::to_string::<Vec<u8>>(&vec![]).unwrap())
    .execute(&mut tx)
    .await
    .unwrap();

    tx.commit().await.unwrap();

    //  0       1               2           3          4       5        6               7
    // view | parent_hash | justify_view | prop_hash | tx | qc.view | qc.node_hash | qc.combined_sign
    let (v, p, j, h, s) =
        sqlx::query("select * from proposal inner join qc on proposal.view=qc.view limit 1;")
            .map(|row: MySqlRow| {
                let s: (u64, String, u64, String, String) =
                    (row.get(0), row.get(1), row.get(2), row.get(3), row.get(7));
                s
            })
            .fetch_one(&db.conn)
            .await
            .unwrap();

    assert!(v == 0 && p == parent_hash && j == 0 && h == node_hash && s == sign);

    let transferred = combined_sign_from_vec_u8(base64::decode(&s).unwrap());
    assert!(
        combined_sign == transferred,
        "\n{:?}\n{:?}",
        combined_sign,
        transferred
    );

    db.close().await;
    Ok(())
}

#[test]
fn test_sqlx_tx() {
    init_logger();
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(test_1())
        .unwrap();
}
