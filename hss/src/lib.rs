//! Hotstuff storage.

// pub mod sqlite;

use cryptokit::{DefaultSignaturer, Signaturer};
use hotstuff_rs::safety::machine::{SafetyStorage, Snapshot};
use hs_data::*;
use log::{debug, error, info};
use pacemaker::{
    data::{combine_time_certificates, BranchData, BranchSyncStrategy, TimeoutCertificate},
    liveness_storage::{LivenessStorage, LivenessStorageErr},
};
use sqlx::{mysql::MySqlRow, Executor, MySqlPool, Row};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::Arc,
    time::SystemTime,
};
use threshold_crypto::serde_impl::SerdeSecret;

use thiserror::Error;
#[derive(Debug, Clone, Error)]
pub enum HotStuffStroageErr {
    #[error("falied to flush, internal err")]
    FailedToFlush,
}

fn map_row_to_proposal(row: MySqlRow) -> (NodeHash, Arc<TreeNode>) {
    let view: ViewNumber = row.get(0);
    let parent_hash: String = row.get(1);
    // let justify_view: ViewNumber = row.get(2);
    // let prop_hash: String = row.get(3);
    let tx: String = row.get(4);
    let node_hash: NodeHash =
        NodeHash::from_vec(&base64::decode(&row.get::<String, _>(6)).unwrap());
    let combined_sign: String = row.get(7);
    let node = Arc::new(TreeNode {
        height: view,
        txs: serde_json::from_str(&tx).unwrap(),
        parent: NodeHash::from_vec(&base64::decode(&parent_hash).unwrap()),
        justify: GenericQC::new(
            view,
            &node_hash,
            &combined_sign_from_vec_u8(base64::decode(&combined_sign).unwrap()),
        ),
    });

    (node_hash, node)
}

async fn get_one_node_by_view(
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

async fn recover_tc_map(
    conn_pool: &MySqlPool,
) -> BTreeMap<ViewNumber, HashSet<TimeoutCertificate>> {
    let mut tc_map = BTreeMap::new();
    sqlx::query("select view, partial_tc from partial_tc;")
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

async fn recover_hotstuff_config(conn_pool: &MySqlPool) -> HotStuffConfig {
    let (token, total, replica_id, addr) =
        sqlx::query("select token, total, self_id, self_addr from hotstuff_conf limit 1; ")
            .map(|row: MySqlRow| {
                let token: String = row.get(0);
                let total: usize = row.get::<u64, _>(1) as usize;
                let replica_id: String = row.get(2);
                let addr: String = row.get(4);

                (token, total, replica_id, addr)
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

async fn recover_hotstuff_state(conn_pool: &MySqlPool) -> InMemoryState {
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

    let (_, leaf) = get_one_node_by_view(conn_pool, leaf_view).await.unwrap();
    let (_, b_locked) = get_one_node_by_view(conn_pool, locked_view).await.unwrap();
    let (_, b_executed) = get_one_node_by_view(conn_pool, executed_view)
        .await
        .unwrap();

    let qc_high = Arc::new(leaf.justify().clone());

    let state = InMemoryState {
        token,
        leaf,
        qc_high,
        vheight,
        current_view,
        committed_height,
        b_executed,
        b_locked,
    };

    state
}

async fn recover_signaturer(conn_pool: &MySqlPool) -> DefaultSignaturer {
    sqlx::query("select pk_set, sk_share, sk_id from crypto where token = ? limit 1; ")
        .map(|row: MySqlRow| {
            let pk_set: String = row.get(0);
            let sk_share: String = row.get(1);
            let sk_id: u64 = row.get(2);

            DefaultSignaturer {
                sign_id: sk_id as usize,
                pks: serde_json::from_str(&pk_set).unwrap(),
                sks: serde_json::from_str::<SerdeSecret<SK>>(&sk_share)
                    .unwrap()
                    .0,
            }
        })
        .fetch_one(conn_pool)
        .await
        .unwrap()
}

async fn create_tables(conn_pool: &MySqlPool) {
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
            `sk_id`    bigint NOT NULL ,

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
            `leaf_view`       bigint NOT NULL ,

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

async fn drop_tables(conn_pool: &MySqlPool) {
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
pub struct InMemoryState {
    // Token of the hotstuff system.
    token: String,

    // Leaf is the node with justify=qc-high.
    leaf: Arc<TreeNode>,

    // Qc-high is the QC with higest view this replica has ever seen.
    qc_high: Arc<GenericQC>,

    // height of last voted proposal this replica has ever seen.
    vheight: ViewNumber,

    current_view: ViewNumber,

    // height for last committed
    committed_height: ViewNumber,

    // Lasted executed proposal.
    b_executed: Arc<TreeNode>,

    // Last locked proposal.
    b_locked: Arc<TreeNode>,
}
pub struct MySQLStorage {
    pub conn_pool: sqlx::MySqlPool,
}

impl MySQLStorage {}

/// Create a HotStuffStorage using in memory storage(Mysql disabled).
pub fn init_in_mem_storage(
    total: usize,
    init_node: &TreeNode,
    init_node_hash: &NodeHash,
    init_qc: &GenericQC,
    self_id: ReplicaID,
    peers_addr: HashMap<ReplicaID, String>,
    signaturer: DefaultSignaturer,
) -> HotstuffStorage {
    let token = format!("hotstuff_test");

    let conf = HotStuffConfig {
        token: token.clone(),
        total,
        replica_id: self_id,
        peers_addr,
    };

    let mut hss = HotstuffStorage::new(token, init_node, init_qc, conf, signaturer, None);
    hss.in_mem_queue
        .insert(init_node_hash.clone(), Arc::new(init_node.clone()));
    hss
}

/// Create a HotStuffStorage with MySQL enabled.
pub async fn init_hotstuff_storage(
    token: String,
    total: usize,
    init_node: &TreeNode,
    init_node_hash: &NodeHash,
    self_id: ReplicaID,
    peers_addr: HashMap<ReplicaID, String>,
    mysql_addr: &str,
    signaturer: DefaultSignaturer,
) -> HotstuffStorage {
    let conn_pool = sqlx::pool::PoolOptions::new()
        .max_connections(4)
        .connect(mysql_addr)
        .await
        .unwrap();

    let backend = Some(MySQLStorage { conn_pool });

    let conf = HotStuffConfig {
        token: token.clone(),
        total,
        replica_id: self_id,
        peers_addr,
    };

    let mut hss = HotstuffStorage::new(
        token,
        &hs_data::INIT_NODE,
        &hs_data::INIT_QC,
        conf,
        signaturer,
        backend,
    );
    hss.init().await;
    hss.in_mem_queue
        .insert(init_node_hash.clone(), Arc::new(init_node.clone()));
    hss
}

/// HotstuffStorage is based on mysql and promises that:
/// - It won't lose any proposal with a QuorumCertificate, once received.
/// - It won't lose any QuorumCertificate and TimeCertificate, once received.
///
/// For example , Persistent Stroage keeps `3<-4` in branch `3<-4<-4` and never lose them.
/// QC carried by the second `4` should be stablized but no guarantees for the second proposal `4` itself.
pub struct HotstuffStorage {
    // None for disable MySQL backend.
    backend: Option<MySQLStorage>,
    state: InMemoryState,
    conf: HotStuffConfig,
    signaturer: DefaultSignaturer,
    tc_map: BTreeMap<ViewNumber, HashSet<TimeoutCertificate>>,

    in_mem_queue: HashMap<NodeHash, Arc<TreeNode>>,

    combined_tc_queue: VecDeque<TimeoutCertificate>,
    partial_tc_queue: VecDeque<TimeoutCertificate>,

    /// consider case: `3<-4<-4`
    /// proposal `3<-4` should be appended into prop_queue and
    /// the second 4's qc should be appended into justify_queue.
    prop_queue: VecDeque<Arc<TreeNode>>,
    justify_queue: VecDeque<GenericQC>,

    dirty: bool,
}

pub struct HotStuffConfig {
    // Hotstuff system token.
    pub token: String,
    // Peers number.
    pub total: usize,
    pub replica_id: ReplicaID,
    pub peers_addr: HashMap<ReplicaID, String>,
}

impl HotStuffConfig {
    pub fn threshold(&self) -> usize {
        (self.total << 1) / 3
    }
}

impl HotstuffStorage {
    pub fn new(
        token: String,
        init_node: &TreeNode,
        init_qc: &GenericQC,
        conf: HotStuffConfig,
        signaturer: DefaultSignaturer,
        backend: Option<MySQLStorage>,
    ) -> Self
    where
        Self: Sized,
    {
        let storage = Self {
            state: InMemoryState {
                token,
                current_view: 0,
                vheight: 0,
                committed_height: 0,
                b_executed: Arc::new(init_node.clone()),
                b_locked: Arc::new(init_node.clone()),
                // safety related
                leaf: Arc::new(init_node.clone()),
                qc_high: Arc::new(init_qc.clone()),
            },
            backend,
            signaturer,
            conf,
            tc_map: BTreeMap::new(),
            in_mem_queue: HashMap::new(),
            combined_tc_queue: VecDeque::with_capacity(8),
            partial_tc_queue: VecDeque::with_capacity(8),
            prop_queue: VecDeque::with_capacity(8),
            justify_queue: VecDeque::with_capacity(8),
            dirty: false,
        };
        // storage.append_new_node(init_node);
        storage
    }

    pub async fn init(&mut self) {
        if self.backend.as_ref().is_none() {
            return;
        }

        let backend = self.backend.as_ref().unwrap();
        drop_tables(&backend.conn_pool).await;
        create_tables(&backend.conn_pool).await;
        let mut tx = backend.conn_pool.begin().await.unwrap();

        // init peers
        for (id, peers) in &self.conf.peers_addr {
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
        .bind(&self.conf.token)
        .bind(self.conf.total as u64)
        .bind(self.conf.peers_addr.get(&self.conf.replica_id).unwrap())
        .bind(&self.conf.replica_id)
        .execute(&mut tx)
        .await
        .unwrap();

        // refactor
        let pk_set = serde_json::to_string(&self.signaturer.pks).unwrap();
        let sk_share: SerdeSecret<SK> = SerdeSecret(self.signaturer.sks.clone());
        let encoded_sks = serde_json::to_string(&sk_share).unwrap();
        debug!("pk set size = {} Byte", pk_set.len());

        // init crypto
        sqlx::query("insert into crypto (token, pk_set, sk_share, sk_id) values (?, ?, ?, ?);")
            .bind(&self.conf.token)
            .bind(&pk_set)
            .bind(&encoded_sks)
            .bind(self.signaturer.sign_id() as u64)
            .execute(&mut tx)
            .await
            .unwrap();

        tx.commit().await.unwrap();
    }

    pub async fn recover(token: String, addr: &str) -> HotstuffStorage {
        // refactor
        // load all proposals;

        let conn_pool = sqlx::MySqlPool::connect(addr).await.unwrap();

        info!("start recovering");

        let st = SystemTime::now();
        let conf = recover_hotstuff_config(&conn_pool).await;
        let mut dur_recover_state = SystemTime::now().duration_since(st).unwrap();
        info!(
            "recover hotstuff configuration, took {} ms",
            dur_recover_state.as_millis()
        );

        let st_1 = SystemTime::now();
        let in_mem_queue = sqlx::query(
            "
            select * from proposal inner join qc 
            on proposal.justify_view=qc.view 
            order by proposal.justify_view;
            ",
        )
        .map(|row: MySqlRow| map_row_to_proposal(row))
        .fetch_all(&conn_pool)
        .await
        .unwrap()
        .into_iter()
        .collect::<HashMap<NodeHash, Arc<TreeNode>>>();

        dur_recover_state = SystemTime::now().duration_since(st_1).unwrap();
        info!(
            "recover in-mem proposal tree, took {} ms",
            dur_recover_state.as_millis()
        );

        let st_2 = SystemTime::now();
        let state = recover_hotstuff_state(&conn_pool).await;
        dur_recover_state = SystemTime::now().duration_since(st_2).unwrap();
        info!(
            "recover in mem hotstuff state, took {} ms",
            dur_recover_state.as_millis()
        );

        let st_3 = SystemTime::now();
        let tc_map = recover_tc_map(&conn_pool).await;
        dur_recover_state = SystemTime::now().duration_since(st_3).unwrap();
        info!(
            "recover tc mapper, took {} ms",
            dur_recover_state.as_millis()
        );

        // recover crypto
        let st_4 = SystemTime::now();
        let signaturer = recover_signaturer(&conn_pool).await;
        dur_recover_state = SystemTime::now().duration_since(st_4).unwrap();
        info!(
            "recover signaturer, took {} ms",
            dur_recover_state.as_millis()
        );

        // update in memory.
        let end = SystemTime::now();
        let recover_dur = end.duration_since(st).unwrap();

        info!(
            "recover done, time consumed: {:.4} s",
            recover_dur.as_secs_f32(),
        );

        HotstuffStorage {
            backend: Some(MySQLStorage { conn_pool }),
            state,
            conf,
            signaturer,
            tc_map,
            in_mem_queue,
            combined_tc_queue: VecDeque::with_capacity(8),
            partial_tc_queue: VecDeque::with_capacity(8),
            prop_queue: VecDeque::with_capacity(8),
            justify_queue: VecDeque::with_capacity(8),
            dirty: false,
        }
    }

    // refactor
    pub async fn async_flush(&mut self) -> Result<(), sqlx::Error> {
        if self.backend.is_none() || !self.dirty {
            return Ok(());
        }

        // start rx

        let mut tx = self.backend.as_ref().unwrap().conn_pool.begin().await?;
        // flush proposal queue;
        debug!(
            "queue size - prop={}, qc={}, ptc={}, ctc={}",
            self.prop_queue.len(),
            self.justify_queue.len(),
            self.partial_tc_queue.len(),
            self.combined_tc_queue.len()
        );
        // flush prop and justify
        while let Some(prop) = self.prop_queue.pop_front() {
            let view = prop.height();
            let parent_hash: String = base64::encode(prop.parent_hash());
            let node_hash = base64::encode(prop.justify().node_hash());
            let sign = base64::encode(prop.justify().combined_sign().to_bytes());
            let txn = serde_json::to_string(&prop.tx()).unwrap();

            // Note that previous flush may insert a qc with the same view.
            sqlx::query(
                "
                    insert ignore into qc
                    (view, node_hash, combined_sign)
                    values
                    (?, ?, ?)
                ;",
            )
            .bind(view)
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
            .bind(view)
            .bind(&parent_hash)
            .bind(view)
            .bind(&node_hash)
            .bind(&txn)
            .execute(&mut tx)
            .await
            .unwrap();
        }

        // flush justify queue;
        while let Some(qc) = self.justify_queue.pop_front() {
            let view = qc.view();
            let node_hash = base64::encode(qc.node_hash());
            let sign = base64::encode(qc.combined_sign().to_bytes());

            // The previous proposal flushing may insert a qc with the same view.
            sqlx::query(
                "
                    insert ignore into qc, 
                    (view, node_hash, combined_sign)
                    values
                    (?, ?, ?)
                ;",
            )
            .bind(view)
            .bind(&node_hash)
            .bind(&sign)
            .execute(&mut tx)
            .await
            .unwrap();
        }

        // todo: flush ptc

        // todo: flush ctc

        // flush state
        sqlx::query(
            "
                replace into hotstuff_state 
                (token, current_view, last_voted_view, locked_view, committed_view, executed_view, leaf_view) 
                values 
                (?, ?, ?, ?, ?, ?, ?);",
        )
        .bind(&self.state.token)
        .bind(self.state.current_view)
        .bind(self.state.vheight)
        .bind(self.state.b_locked.height())
        .bind(self.state.committed_height)
        .bind(self.state.b_executed.height())
        .bind(self.state.leaf.height())
        .execute(&mut tx)
        .await
        .unwrap();

        let res = tx.commit().await;
        info!("flush ");
        self.reset_dirty();
        res
    }

    pub fn threshold(&self) -> usize {
        (self.conf.total << 1) / 3
    }

    fn compress_tc(&mut self) -> Option<ViewNumber> {
        // compress
        let view = self
            .tc_map
            .iter()
            .filter(|(k, s)| s.len() >= self.conf.threshold())
            .max_by_key(|(k, _)| *k)
            .map(|(view, _)| *view);

        if let Some(v) = view {
            let from = String::default();
            self.tc_map
                .remove(&v)
                .and_then(|tc| {
                    combine_time_certificates(
                        &self.signaturer,
                        &from,
                        v,
                        self.get_qc_high().as_ref().clone(),
                        &tc,
                    )
                })
                .map_or((), |combined_tc| {
                    self.combined_tc_queue.push_back(combined_tc);
                });
        }
        view
    }

    fn get(&self, node_hash: &NodeHash) -> Option<Arc<TreeNode>> {
        // refactor
        match self.in_mem_queue.get(node_hash) {
            None => None,
            Some(s) => Some(s.clone()),
        }
    }

    fn insert(&mut self, node_hash: NodeHash, prop: Arc<TreeNode>) {
        self.in_mem_queue.insert(node_hash.clone(), prop.clone());
        self.prop_queue.push_back(prop);
    }

    #[inline]
    fn set_dirty(&mut self) {
        self.dirty = true;
    }

    #[inline]
    fn reset_dirty(&mut self) {
        self.dirty = false;
    }
}

#[async_trait::async_trait]
impl SafetyStorage for HotstuffStorage {
    async fn flush(&mut self) -> hotstuff_rs::safety::machine::Result<()> {
        self.async_flush()
            .await
            .map_err(|e| hotstuff_rs::safety::machine::SafetyErr::InternalErr(Box::new(e)))
    }

    fn append_new_node(&mut self, node: &TreeNode) {
        let h = TreeNode::hash(node);
        let node = Arc::new(node.clone());

        // Warning: OOM since every node has a replica in memory.
        self.insert(h, node);
        self.set_dirty();
    }

    fn append_new_qc(&mut self, qc: &GenericQC) {
        self.justify_queue.push_back(qc.clone());
        self.set_dirty();
    }

    fn find_three_chain(&self, node: &TreeNode) -> Vec<Arc<TreeNode>> {
        let mut chain = Vec::with_capacity(3);
        if let Some(b3) = self.get(node.justify().node_hash()) {
            chain.push(b3.clone());
            if let Some(b2) = self.get(b3.justify().node_hash()) {
                chain.push(b2.clone());
                if let Some(b1) = self.get(b2.justify().node_hash()) {
                    chain.push(b1.clone());
                }
            }
        }
        chain
    }

    /// Persistent
    fn update_leaf(&mut self, new_leaf: &TreeNode) {
        self.state.vheight = new_leaf.height();
        self.state.leaf = Arc::new(new_leaf.clone());
        self.in_mem_queue
            .insert(TreeNode::hash(new_leaf), self.state.leaf.clone());
        self.set_dirty();
    }

    fn get_leaf(&self) -> Arc<TreeNode> {
        self.state.leaf.clone()
    }

    fn get_qc_high(&self) -> Arc<GenericQC> {
        self.state.qc_high.clone()
    }

    /// qc_high.node == qc_node.
    fn update_qc_high(&mut self, new_qc_node: &TreeNode, new_qc_high: &GenericQC) {
        if let Some(qc_node) = self.get(self.get_qc_high().node_hash()) {
            if new_qc_node.height() > qc_node.height() {
                self.state.qc_high = Arc::new(new_qc_high.clone());
                // self.vheight = new_qc_node.height();
                self.update_leaf(new_qc_node);

                debug!("update qc-high(h={})", new_qc_node.height());
                self.set_dirty();
            }
        }
    }

    fn is_conflicting(&self, a: &TreeNode, b: &TreeNode) -> bool {
        let (a, b) = if a.height() >= b.height() {
            (a, b)
        } else {
            (b, a)
        };

        // a.height() >= b.height()
        let mut height_now = a.height();
        let mut parent_hash = a.parent_hash().clone();

        // refactor
        let mut node: Arc<TreeNode> = Arc::new(a.clone());
        while height_now > b.height() {
            if let Some(prev) = self.get(&parent_hash) {
                node = prev.clone();
                height_now = prev.height();
                // if prev == init_qc, then prev.parent() points to an invalied node.
                parent_hash = prev.parent_hash().clone();
            } else {
                break;
            }
        }

        TreeNode::hash(node.as_ref()) != TreeNode::hash(b)
    }

    fn get_node(&self, node_hash: &NodeHash) -> Option<Arc<TreeNode>> {
        self.get(node_hash).and_then(|node| Some(node.clone()))
    }

    fn get_locked_node(&self) -> Arc<TreeNode> {
        self.state.b_locked.clone()
    }

    // Persistent
    fn update_locked_node(&mut self, node: &TreeNode) {
        debug!("locked at node with height {}", node.height());
        self.state.b_locked = Arc::new(node.clone());
        self.set_dirty();
    }

    fn get_last_executed(&self) -> Arc<TreeNode> {
        self.state.b_executed.clone()
    }

    // Persistent
    fn update_last_executed_node(&mut self, node: &TreeNode) {
        self.state.b_executed = Arc::new(node.clone());
        self.set_dirty();
    }

    fn get_view(&self) -> ViewNumber {
        self.state.current_view
    }

    // Persistent
    fn increase_view(&mut self, new_view: ViewNumber) {
        self.state.current_view = ViewNumber::max(self.state.current_view, new_view);
        self.set_dirty();
    }

    fn is_consecutive_three_chain(&self, chain: &Vec<impl AsRef<TreeNode>>) -> bool {
        if chain.len() != 3 {
            debug!("not consecutive 3-chain, len={}", chain.len());
            return false;
        }

        let b_3 = chain.get(0).unwrap().as_ref();
        let b_2 = chain.get(1).unwrap().as_ref();
        let b = chain.get(2).unwrap().as_ref();

        let pred_32 = b_3.parent_hash() == &TreeNode::hash(b_2);
        let pred_21 = b_2.parent_hash() == &TreeNode::hash(b);
        // &b_3.parent_hash() == &TreeNode::hash(b_2) && &b_2.parent_hash() == &TreeNode::hash(b)
        debug!(
            "consecutive judge with h = {},{},{}: {} - {}",
            b_3.height(),
            b_2.height(),
            b.height(),
            pred_32,
            pred_21
        );
        pred_32 && pred_21
    }

    fn get_vheight(&self) -> ViewNumber {
        self.state.vheight
    }

    // Persistent
    fn update_vheight(&mut self, vheight: ViewNumber) -> ViewNumber {
        let prev = self.state.vheight;
        self.state.vheight = ViewNumber::max(self.state.vheight, vheight);
        self.set_dirty();

        prev
    }

    // TODO: add informer for watchers.
    fn commit(&mut self, to_commit: &TreeNode) {
        if self.state.committed_height >= to_commit.height() {
            debug!("to_commit with smaller height {}", to_commit.height());
            return;
        }
        let to_committed_height = to_commit.height();
        for h in self.state.committed_height + 1..=to_committed_height {
            // TODO:execute,
            self.state.committed_height = h;
        }
        self.state.b_executed = Arc::new(to_commit.clone());
        self.set_dirty();

        info!(
            "commit new proposal, committed_height = {}",
            self.state.committed_height
        );
    }

    fn hotstuff_status(&self) -> Snapshot {
        Snapshot {
            view: self.state.current_view,
            leader: None,
            qc_high: Box::new(self.state.qc_high.as_ref().clone()),
            leaf: Box::new(self.state.leaf.as_ref().clone()),
            locked_node: Box::new(self.state.b_locked.as_ref().clone()),
            last_committed: self.state.committed_height,
        }
    }
}

#[async_trait::async_trait]
impl LivenessStorage for HotstuffStorage {
    async fn flush(&mut self) -> Result<(), LivenessStorageErr> {
        self.async_flush()
            .await
            .map_err(|_| LivenessStorageErr::FailedToFlush)
    }

    fn append_tc(&mut self, tc: TimeoutCertificate) -> Result<ViewNumber, LivenessStorageErr> {
        match tc.view_sign() {
            pacemaker::data::SignType::Partial(_) => {
                self.partial_tc_queue.push_back(tc);
            }
            pacemaker::data::SignType::Combined(_) => {
                self.combined_tc_queue.push_back(tc);
            }
        }
        self.set_dirty();

        // current_view - 1 is ok
        Ok(self
            .compress_tc()
            .unwrap_or(self.get_view().saturating_sub(1)))
    }

    fn is_reach_threshold(&self, view: ViewNumber) -> bool {
        self.tc_map
            .get(&view)
            .map_or(false, |s| s.len() >= self.conf.threshold())
    }

    fn update_qc_high(
        &mut self,
        node: &TreeNode,
        qc: &GenericQC,
    ) -> Result<(), LivenessStorageErr> {
        <HotstuffStorage as SafetyStorage>::update_qc_high(self, node, qc);
        self.set_dirty();

        Ok(())
    }

    fn fetch_branch(
        &self,
        strategy: &BranchSyncStrategy,
    ) -> Result<BranchData, LivenessStorageErr> {
        // refactor
        // Find proposals in memory through
        match strategy {
            BranchSyncStrategy::Grow {
                grow_from,
                batch_size,
            } => {
                let v: Vec<TreeNode> = self
                    .in_mem_queue
                    .values()
                    .filter(|s| &s.height() > grow_from)
                    .map(|node| node.as_ref().clone())
                    .collect();
                Ok(BranchData { data: v })
            }
        }
    }

    fn is_qc_node_exists(&mut self, qc: &GenericQC) -> bool {
        self.get(&qc.node_hash()).is_some()
    }

    fn get_locked_node(&mut self) -> &TreeNode {
        self.state.b_locked.as_ref()
    }

    fn get_leaf(&self) -> &TreeNode {
        self.state.leaf.as_ref()
    }

    fn increase_view(&mut self, new_view: ViewNumber) {
        <Self as SafetyStorage>::increase_view(self, new_view);
    }
}
