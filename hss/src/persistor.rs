use std::{collections::VecDeque, sync::Arc};

use cryptokit::DefaultSignaturer;
use hs_data::{GenericQC, TreeNode};
use log::{debug, info};
use pacemaker::data::TimeoutCertificate;
use sqlx::MySqlPool;
use tokio::sync::{
    mpsc::{channel, Receiver, Sender},
    oneshot,
};

use crate::{
    recover::{create_tables, drop_tables, persist_other_msg},
    HotStuffConfig, InMemoryState,
};
pub struct MySQLStorage {
    conn_pool: Option<MySqlPool>,
    sender: Sender<FlushTask>,
    recvr_tmp: Option<Receiver<FlushTask>>,
}

impl MySQLStorage {
    pub async fn init(&mut self, conf: &HotStuffConfig, signaturer: &DefaultSignaturer) {
        if let Some(conn_pool) = self.conn_pool.take() {
            drop_tables(&conn_pool).await;
            create_tables(&conn_pool).await;
            persist_other_msg(&conn_pool, conf, signaturer).await;

            let recvr = self.recvr_tmp.take().unwrap();
            let flusher = MySQLFlusher { conn_pool, recvr };
            tokio::spawn(flusher.run());
        }
    }

    pub fn new(conn: MySqlPool) -> MySQLStorage {
        let (sender, recvr) = channel(4);
        Self {
            conn_pool: Some(conn),
            sender,
            recvr_tmp: Some(recvr),
        }
    }

    pub async fn async_flush(&mut self, task: FlushTask) {
        let _ = self.sender.send(task).await;
    }

    pub fn connection(&self) -> Option<&MySqlPool> {
        self.conn_pool.as_ref()
    }
}

pub struct DirtyData {
    combined_tc_queue: VecDeque<TimeoutCertificate>,
    partial_tc_queue: VecDeque<TimeoutCertificate>,
    prop_queue: VecDeque<Arc<TreeNode>>,
    justify_queue: VecDeque<GenericQC>,
    state: InMemoryState,
}

pub enum FlushTask {
    Async {
        data: DirtyData,
    },
    Sync {
        data: DirtyData,
        done: oneshot::Sender<()>,
    },
}

impl FlushTask {
    pub fn synch(
        combined_tc_queue: VecDeque<TimeoutCertificate>,
        partial_tc_queue: VecDeque<TimeoutCertificate>,
        prop_queue: VecDeque<Arc<TreeNode>>,
        justify_queue: VecDeque<GenericQC>,
        state: InMemoryState,
        done: oneshot::Sender<()>,
    ) -> Self {
        FlushTask::Sync {
            data: DirtyData {
                combined_tc_queue,
                partial_tc_queue,
                prop_queue,
                justify_queue,
                state,
            },
            done,
        }
    }

    pub fn asynch(
        combined_tc_queue: VecDeque<TimeoutCertificate>,
        partial_tc_queue: VecDeque<TimeoutCertificate>,
        prop_queue: VecDeque<Arc<TreeNode>>,
        justify_queue: VecDeque<GenericQC>,
        state: InMemoryState,
    ) -> Self {
        FlushTask::Async {
            data: DirtyData {
                combined_tc_queue,
                partial_tc_queue,
                prop_queue,
                justify_queue,
                state,
            },
        }
    }
}

struct MySQLFlusher {
    recvr: Receiver<FlushTask>,
    conn_pool: sqlx::MySqlPool,
}

impl MySQLFlusher {
    async fn run(mut self) -> std::io::Result<()> {
        while let Some(task) = self.recvr.recv().await {
            match task {
                FlushTask::Async { mut data } => self.flush(&mut data).await?,
                FlushTask::Sync { mut data, done } => {
                    self.flush(&mut data).await?;
                    if done.send(()).is_err() {
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    async fn flush(&mut self, data: &mut DirtyData) -> std::io::Result<()> {
        // start rx
        let mut tx = self.conn_pool.begin().await.unwrap();
        // flush proposal queue;
        debug!(
            "queue size - prop={}, qc={}, ptc={}, ctc={}",
            data.prop_queue.len(),
            data.justify_queue.len(),
            data.partial_tc_queue.len(),
            data.combined_tc_queue.len()
        );
        // flush prop and justify
        while let Some(prop) = data.prop_queue.pop_front() {
            let view = prop.height();
            let parent_hash: String = base64::encode(prop.parent_hash());
            let node_hash = base64::encode(TreeNode::hash(&prop));
            let sign = base64::encode(prop.justify().combined_sign().to_bytes());
            let txn = serde_json::to_vec(prop.tx()).unwrap();

            // Note that previous flush may insert a qc with the same view.
            sqlx::query(
                "
                    insert ignore into qc
                    (view, node_hash, combined_sign)
                    values
                    (?, ?, ?)
                ;",
            )
            .bind(prop.justify().view())
            .bind(&node_hash)
            .bind(&sign)
            .execute(&mut tx)
            .await
            .unwrap();

            sqlx::query(
                "
                    insert ignore into proposal
                    (view, parent_hash, justify_view, prop_hash, txn)
                    values
                    (?, ?, ?, ?, ?)
                ;",
            )
            .bind(view)
            .bind(&parent_hash)
            .bind(prop.justify().view())
            .bind(&node_hash)
            .bind(&txn)
            .execute(&mut tx)
            .await
            .unwrap();
        }

        // flush justify queue;
        while let Some(qc) = data.justify_queue.pop_front() {
            let view = qc.view();
            let node_hash = base64::encode(qc.node_hash());
            let sign = base64::encode(qc.combined_sign().to_bytes());

            // The previous proposal flushing may insert a qc with the same view.
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
        }

        // todo: flush ptc
        while let Some(ptc) = data.partial_tc_queue.pop_front() {
            sqlx::query(
                "
                insert ignore into partial_tc
                (view, replica_id, partial_sign)
                values
                (?, ?, ?)
                ",
            )
            .bind(ptc.view())
            .bind(ptc.from())
            .bind(serde_json::to_string(ptc.view_sign()).unwrap())
            .execute(&mut tx)
            .await
            .unwrap();
        }

        // todo: flush ctc
        while let Some(ctc) = data.combined_tc_queue.pop_front() {
            sqlx::query(
                "
                insert ignore into combined_tc
                (view, combined_sign)
                values
                (?, ?);
                ",
            )
            .bind(ctc.view())
            .bind(serde_json::to_string(ctc.view_sign()).unwrap())
            .execute(&mut tx)
            .await
            .unwrap();
        }

        // flush state
        sqlx::query(
            "
                replace into hotstuff_state 
                (token, current_view, last_voted_view, locked_view, committed_view, executed_view, leaf_view) 
                values 
                (?, ?, ?, ?, ?, ?, ?);",
        )
        .bind(&data.state.token)
        .bind(data.state.current_view)
        .bind(data.state.vheight)
        .bind(data.state.b_locked.height())
        .bind(data.state.committed_height)
        .bind(data.state.last_commit.height())
        .bind(data.state.leaf.height())
        .execute(&mut tx)
        .await
        .unwrap();

        let _ = tx.commit().await;
        info!("flushed ");

        Ok(())
    }
}
