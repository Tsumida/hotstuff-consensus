//! Mocker for testing.
use std::collections::HashMap;
use std::sync::Arc;
use std::{mem::MaybeUninit, unimplemented, vec};

use hss::InMemoryStorage;
use log::debug;
use simplelog::{CombinedLogger, Config, LevelFilter, WriteLogger};
use threshold_crypto::{PublicKeySet, SecretKeySet, SecretKeyShare, Signature, SignatureShare};

use cryptokit::DefaultSignaturer;
use hs_data::*;

use hotstuff_rs::safety::{
    machine::{Machine, Ready, Safety, SafetyErr, SafetyEvent},
    voter::Voter,
};
use hs_data::msg::Context;

const DEBUG_MODE: bool = false;

pub(crate) fn init_logger() {
    if DEBUG_MODE {
        let _ = CombinedLogger::init(vec![
            //TermLogger::new(LevelFilter::Debug, Config::default(), TerminalMode::Mixed),
            WriteLogger::new(
                LevelFilter::Debug,
                Config::default(),
                std::fs::File::create("./test-output/my_rust_bin.log").unwrap(),
            ),
        ]);
    }
}

pub(crate) fn threshold_sign_kit(
    n: usize,
    t: usize,
) -> (SecretKeySet, PublicKeySet, Vec<(usize, SecretKeyShare)>) {
    assert!(t <= n);

    let s = SecretKeySet::random(t, &mut rand::thread_rng());

    let vec_sk = (0..n).map(|i| (i, s.secret_key_share(i))).collect();
    let pks = s.public_keys();
    (s, pks, vec_sk)
}

pub(crate) enum ExpectedState<'a> {
    // for hotstuff status
    LockedAt(String),
    CommittedBeforeHeight(usize),
    QcHighOf(String),

    // for proposal
    ParentIs(String, &'a TreeNode),
    QcOf(String, &'a TreeNode),
}

pub(crate) struct MockHotStuff {
    testee: Option<Machine<InMemoryStorage>>,

    pks: Option<PublicKeySet>,
    sk: Option<SecretKeySet>,
    sks: Vec<(usize, SecretKeyShare)>,

    tx_to_hash: HashMap<String, NodeHash>,

    nodes: HashMap<NodeHash, Arc<TreeNode>>,
    qcs: HashMap<QCHash, Arc<GenericQC>>,
    parent: NodeHash,
    //
    qc_high: Arc<GenericQC>,
    height: u64,
    th: usize,
    n: usize,
    leader_id: usize,
    testee_id: usize,
    init_done: bool,

    adversial: Option<usize>,
}

impl MockHotStuff {
    pub(crate) fn new(n: usize) -> Self {
        let init_qc = Arc::new((*INIT_QC).clone());
        let init_node_hash = (*INIT_NODE_HASH).clone();

        Self {
            testee: None,
            parent: init_node_hash,
            qc_high: init_qc,

            pks: None,
            sk: None,
            sks: Vec::new(),
            tx_to_hash: HashMap::new(),

            nodes: HashMap::new(),
            qcs: HashMap::new(),

            height: 0,
            th: 0,
            n,
            leader_id: 0,
            testee_id: 0,

            init_done: false,
            adversial: None,
        }
    }

    pub(crate) fn specify_leader(&mut self, leader: usize) -> &mut Self {
        assert!(leader < self.n);
        self.leader_id = leader;
        self
    }

    pub(crate) fn specify_testee(&mut self, testee: usize) -> &mut Self {
        assert!(testee < self.n);
        self.testee_id = testee;
        self
    }

    pub(crate) fn specify_adversial(&mut self, adversial: usize) -> &mut Self {
        assert!(adversial < self.n);
        self.adversial = Some(adversial);
        self
    }

    /// Initialize state.
    pub(crate) fn init(&mut self) {
        assert!(!self.init_done);
        assert!(self.n > 0);
        assert!(self.testee_id < self.n);

        self.th = (self.n << 1) / 3;
        let (a, b, c) = threshold_sign_kit(self.n, self.th);
        self.sk = Some(a);
        self.pks = Some(b);
        self.sks = c;

        let init_node = Arc::new((*INIT_NODE).clone());
        let init_qc = Arc::new((*INIT_QC).clone());
        let init_node_hash = (*INIT_NODE_HASH).clone();
        let init_qc_hash = (*INIT_QC_HASH).clone();

        self.tx_to_hash
            .insert(format!("init"), init_node_hash.clone());
        self.qcs.insert(init_qc_hash.clone(), init_qc.clone());
        self.nodes.insert(init_node_hash.clone(), init_node.clone());

        let view = 0;
        let mut node_pool = HashMap::new();
        node_pool.insert(init_node_hash.clone(), init_node.clone());
        let mut qc_map = HashMap::new();
        qc_map.insert(init_qc_hash.clone(), init_qc.clone());

        self.parent = init_node_hash;
        self.qc_high = init_qc;

        let voter = Voter::new(
            self.th,
            DefaultSignaturer::new(
                self.testee_id,
                self.pks.as_ref().unwrap().clone(),
                self.sk.as_ref().unwrap().secret_key_share(self.testee_id),
            ),
        );
        let storage =
            InMemoryStorage::new(node_pool, qc_map, view, &init_node, self.qc_high.as_ref());
        let testee: Machine<InMemoryStorage> = Machine::new(
            voter,
            format!("{}", self.testee_id),
            self.n,
            Some(format!("{}", self.leader_id)),
            storage,
        );

        self.testee = Some(testee);
        self.init_done = true;
    }

    pub(crate) fn testee(&mut self) -> &mut dyn Safety {
        self.testee.as_mut().unwrap()
    }

    /// Construct chain for testee. Note that no qc of last proposal is formed!
    /// For example `a1 <-qc- a2 <-qc- a3`, a1 has 2 qc, a2 has one, but qc of a3 isn't formed.
    pub(crate) fn testee_load_consecutive_proposals(&mut self, cmds: Vec<String>) -> &mut Self {
        let nodes = self.prepare_proposals(&cmds);
        for (cmd, (node, hash)) in cmds.into_iter().zip(nodes) {
            // send proposal to testee
            let res = self.testee_recv_honest_proposal(node.as_ref(), node.justify());
            assert!(res.is_ok(), format!("{:?}", res));

            self.update(
                cmd,
                node.height(),
                Arc::new(node.justify().clone()),
                node,
                hash,
            );
        }

        self
    }

    /// Assertion about internal state of hotstuff module.
    /// Panic if assertion failed.  
    pub(crate) fn check_hotstuff_state_with(&self, expected: &ExpectedState) {
        let ss = self.testee.as_ref().unwrap().take_snapshot();
        match expected {
            ExpectedState::LockedAt(tx) => {
                let node_hash = self.tx_to_hash.get(tx).unwrap();
                let node = self.nodes.get(node_hash).unwrap();
                assert!(
                    ss.locked_node.height() == node.height(),
                    format!("{:?}", node)
                );
            }
            ExpectedState::CommittedBeforeHeight(h) => {
                assert!(ss.last_committed >= *h as u64, format!("{:?}", ss));
            }
            ExpectedState::QcHighOf(tx) => {
                let prop_hash = self.tx_to_hash.get(tx).unwrap();
                let (qc_node_hash, _) = self.nodes.get_key_value(ss.qc_high.node_hash()).unwrap();
                assert_eq!(qc_node_hash, prop_hash);
            }
            _ => unimplemented!(),
        };
    }

    pub(crate) fn sync_state(&mut self, branch: impl IntoIterator<Item = TreeNode>) {
        let branch: Vec<TreeNode> = branch.into_iter().collect();

        // update mocker state.
        for node in &branch {
            self.update(
                String::from_utf8(node.tx().first().unwrap().0.clone()).unwrap(),
                node.height(),
                Arc::new(node.justify().clone()),
                Box::new(node.clone()),
                Box::new(TreeNode::hash(node)),
            );
        }

        let leader = self.leader_id;
        self.testee()
            .process_safety_event(SafetyEvent::BranchSync(
                Context {
                    from: format!("{}", leader),
                    view: 6,
                },
                branch,
            ))
            .unwrap();
    }

    /// let testee extends branch from specified parent`. This method will increate height.
    /// Panic if parent didn't exist.
    pub(crate) fn extend_from(&mut self, parent: String, tx: String) {
        let parent_hash = self.tx_to_hash.get(&parent).unwrap().clone();
        let parent = self.nodes.get(&parent_hash).unwrap().clone();
        // form qc of parent and use it to create new proposal.
        let qc = self.form_qc(self.height, &parent, &parent_hash);

        self.tick();
        let res = self.propose(&parent_hash, tx, qc);
        assert!(if let Ok(Ready::Signature(..)) = res {
            true
        } else {
            false
        });
    }

    /// Send new proposal with corrupted qc to testee. This method will increate height.
    pub(crate) fn propose_with_corrupted_qc(&mut self, parent: String, tx: String) {
        let parent_hash = self.tx_to_hash.get(&parent).unwrap().clone();

        // form qc of parent and use it to create new proposal.
        let qc = self.form_corrupted_qc(self.height, &parent_hash);

        self.tick();
        let res = self.propose(&parent_hash, tx, qc);

        assert!(if let Err(SafetyErr::CorruptedQC) = res {
            true
        } else {
            false
        });
    }

    /// Leader recv txs from other
    pub(crate) fn testee_recv_new_view_msgs(&mut self, txs: Vec<(usize, String)>) -> &mut Self {
        for (i, tx) in txs.iter() {
            let node_hash = self.tx_to_hash.get(tx).unwrap();
            debug!("{}, {:?}", tx, &node_hash);
            let node = self.nodes.get(node_hash).unwrap();
            let qc = node.justify();
            let _ = self
                .testee
                .as_mut()
                .unwrap()
                .process_safety_event(SafetyEvent::RecvNewViewMsg(
                    Context {
                        from: format!("{}", i),
                        view: self.height,
                    },
                    Arc::new(qc.clone()),
                ))
                .unwrap();
        }
        self
    }

    /// As leader, testee propose new proposal.
    pub(crate) fn testee_make_proposal(&mut self, new_tx: String) -> Ready {
        self.tick();

        let res = self
            .testee
            .as_mut()
            .unwrap()
            .process_safety_event(SafetyEvent::NewTx(vec![Txn::new(new_tx.clone())]))
            .unwrap();

        // update mocker
        match &res {
            Ready::NewProposal(_, node) => {
                let hash = Box::new(TreeNode::hash(&node));
                self.update(
                    new_tx,
                    self.height,
                    Arc::new(node.justify().clone()),
                    Box::new(node.as_ref().clone()),
                    hash,
                );
            }
            _ => panic!(),
        }
        res
    }

    pub(crate) fn check_proposal_with(&self, expected: &ExpectedState) -> &Self {
        match expected {
            ExpectedState::ParentIs(parent, prop) => {
                let parent = self.tx_to_hash.get(parent).unwrap();
                assert_eq!(parent, prop.parent_hash());
            }
            ExpectedState::QcOf(qc_node_tx, prop) => {
                let qc_node_hash = self.tx_to_hash.get(qc_node_tx).unwrap();
                let prop_justify_node_hash = prop.justify().node_hash();
                assert_eq!(qc_node_hash, prop_justify_node_hash);
            }

            _ => panic!("invalided expected state for this method"),
        }
        self
    }

    /// Recv one corrupted new-view msg.
    pub(crate) fn testee_recv_corrupted_view_msg(&mut self, qc_node: String) -> &mut Self {
        let prev_qc_hash = self.tx_to_hash.get(&qc_node).unwrap();
        let prev_qc = self.form_corrupted_qc(self.height, prev_qc_hash);
        let _ = self
            .testee
            .as_mut()
            .unwrap()
            .process_safety_event(SafetyEvent::RecvNewViewMsg(
                Context {
                    from: format!("{}", self.adversial.unwrap()),
                    view: self.height,
                },
                prev_qc,
            ));
        self
    }

    pub(crate) fn testee_recv_votes(&mut self, votes: Vec<MockEvent>) -> &mut Self {
        for vote in votes {
            match vote {
                MockEvent::AcceptedVote(from, tx) => {
                    let node_hash = self.tx_to_hash.get(&tx).unwrap();
                    let node = self.nodes.get(node_hash).unwrap();
                    let sign = self
                        .sk
                        .as_ref()
                        .unwrap()
                        .secret_key_share(from)
                        .sign(&node.to_be_bytes());

                    let _ = self.testee.as_mut().unwrap().on_recv_vote(
                        &Context {
                            from: format!("{}", from),
                            view: self.height,
                        },
                        &node,
                        &SignKit::from((sign, from)),
                    );
                }
                MockEvent::CorruptedVote(from, tx) => {
                    let node_hash = self.tx_to_hash.get(&tx).unwrap();
                    let node = self.nodes.get(node_hash).unwrap();
                    let sign = self.form_corrupted_sign();
                    let err = self.testee.as_mut().unwrap().on_recv_vote(
                        &Context {
                            from: format!("{}", from),
                            view: self.height,
                        },
                        &node,
                        &SignKit::from((sign, from)),
                    );

                    assert!(err.is_err());
                }
            };
        }
        self
    }

    /// Make proposals. MockHotstuff will not save this proposal like they were never proposed.
    pub(crate) fn prepare_proposals(
        &self,
        cmds: &Vec<String>,
    ) -> Vec<(Box<TreeNode>, Box<NodeHash>)> {
        let mut prev_qc = self.qc_high.clone();
        let mut props = Vec::with_capacity(cmds.len());
        let mut height = self.height;
        let mut parent = Box::new(self.parent.clone());
        for cmd in cmds {
            height += 1;
            // make node
            let (node, hash) =
                TreeNode::node_and_hash(vec![&Txn::new(cmd.as_bytes())], height, &parent, &prev_qc);

            // make qc with signs from replicas,
            prev_qc = self.form_qc(height, &node, &hash);
            parent = hash;

            props.push((node, parent.clone()));
        }

        props
    }

    /// Increase both testee and mocker's ticker.
    fn tick(&mut self) {
        self.height += 1;
        self.testee
            .as_mut()
            .unwrap()
            .on_view_change(format!("{}", self.leader_id), self.height)
            .unwrap();
    }

    fn form_corrupted_sign(&self) -> Sign {
        unsafe { MaybeUninit::uninit().assume_init() }
    }

    fn propose(
        &mut self,
        parent_hash: &NodeHash,
        tx: String,
        qc: Arc<GenericQC>,
    ) -> Result<Ready, SafetyErr> {
        // parent <- qc <- new_node
        // form qc of parent and use it to create new proposal.
        // let qc = self.form_qc(self.height, &parent_hash);
        self.tick();
        let (node, hash) = TreeNode::node_and_hash(
            vec![&Txn::new(tx.as_bytes())],
            self.height,
            parent_hash,
            &qc,
        );
        let res = self.testee_recv_honest_proposal(&node, &qc);
        self.update(tx, self.height, qc, node, hash);
        res
    }

    fn testee_recv_honest_proposal(
        &mut self,
        node: &TreeNode,
        justify: &GenericQC,
    ) -> Result<Ready, SafetyErr> {
        self.testee.as_mut().unwrap().on_recv_proposal(
            &Context {
                from: format!("mocker"),
                view: self.height as u64,
            },
            node,
            justify,
        )
    }

    /// Create quorum certificate for a node.
    fn form_qc(&self, view: ViewNumber, node: &TreeNode, node_hash: &NodeHash) -> Arc<GenericQC> {
        // make qc with signs from replicas,
        let node_bytes = node.to_be_bytes();
        let signs = self
            .sks
            .iter()
            .map(|(i, sk)| (*i, sk.sign(&node_bytes)))
            .collect::<Vec<(usize, SignatureShare)>>();
        let combined_sign = self
            .pks
            .as_ref()
            .unwrap()
            .combine_signatures(signs.iter().map(|(i, s)| (*i, s)))
            .unwrap();

        Arc::new(GenericQC::new(view, node_hash, &combined_sign))
    }

    fn form_corrupted_qc(&self, view: ViewNumber, node_hash: &NodeHash) -> Arc<GenericQC> {
        let combined_sign: Signature = unsafe { MaybeUninit::uninit().assume_init() };

        Arc::new(GenericQC::new(view, node_hash, &combined_sign))
    }

    fn update(
        &mut self,
        cmd: String,
        view: ViewNumber,
        prev_qc: Arc<GenericQC>,
        node: Box<TreeNode>,
        hash: Box<NodeHash>,
    ) {
        self.tx_to_hash.insert(cmd, hash.as_ref().clone());
        self.height = u64::max(self.height, view);
        self.parent = *hash;
        self.qc_high = prev_qc.clone();
        self.nodes
            .insert(TreeNode::hash(node.as_ref()), Arc::new(*node));
        self.qcs
            .insert(GenericQC::hash(prev_qc.as_ref()), self.qc_high.clone());
    }
}

pub(crate) enum MockEvent {
    // from, tx
    CorruptedVote(usize, String),
    AcceptedVote(usize, String),
}
