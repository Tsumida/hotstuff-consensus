//! Mocker for testing.
use std::unimplemented;

use threshold_crypto::{PublicKeySet, SecretKeySet, SecretKeyShare, SignatureShare};

use crate::{safety::basic::*, utils::threshold_sign_kit};

pub enum ExpectedState {
    LockedInHeight(usize),
    CommittedBeforeHeight(usize),
}

pub struct MockHotStuff {
    pks: Option<PublicKeySet>,
    sk: Option<SecretKeySet>,
    sks: Vec<(usize, SecretKeyShare)>,
    cmds: Vec<Txn>,

    th: usize,
    n: usize,
    leader_id: usize,
    testee: usize,
}

impl MockHotStuff {
    pub fn new(n: usize) -> Self {
        Self {
            pks: None,
            sk: None,
            sks: Vec::new(),
            cmds: Vec::new(),

            th: 0,
            n,
            leader_id: 0,
            testee: 0,
        }
    }

    pub fn specify_leader(&mut self, leader: usize) -> &mut Self {
        assert!(leader < self.n);
        self.leader_id = leader;
        self
    }

    pub fn specify_testee(&mut self, testee: usize) -> &mut Self {
        assert!(testee < self.n);
        self.testee = testee;
        self
    }

    pub fn done(&mut self) {
        assert!(self.n > 0);
        assert!(self.testee < self.n);

        self.th = (self.n << 1) / 3;
        let (a, b, c) = threshold_sign_kit(self.n, self.th);
        self.sk = Some(a);
        self.pks = Some(b);
        self.sks = c;
    }

    pub fn load_continue_chain(&mut self, chain: Vec<Txn>) -> &mut Self {
        unimplemented!()
    }

    pub fn check_with_expected_state(&self, expected: &ExpectedState) -> bool {
        unimplemented!()
    }

    /// extend branch from current leaf.
    pub fn propose(&mut self, tx: Txn) {
        unimplemented!()
    }

    /// extend branch from specified parent`.
    pub fn extend_from(&mut self, parent: Txn, tx: Txn) {
        unimplemented!()
    }
}

/// Build blockchain
pub fn mock_chain(pks: &PublicKeySet, vec_sk: &Vec<(usize, SecretKeyShare)>, cmds: &Vec<Txn>) {
    let first_node = TreeNode::genesis();
    let first_qc = GenericQC::genesis(0, &first_node);
    let mut nodes = Vec::with_capacity(cmds.len());
    let mut qcs = Vec::with_capacity(cmds.len());

    let mut view = 1;
    let mut prev_qc = Box::new(GenericQC::hash(&first_qc));
    let mut parent = Box::new(TreeNode::hash(&first_node));

    for cmd in cmds {
        // make node
        let (node, hash) = TreeNode::node_and_hash(vec![cmd], view, &parent, &prev_qc);

        let signs = vec_sk
            .iter()
            .map(|(i, sk)| (*i, sk.sign(hash.as_ref())))
            .collect::<Vec<(usize, SignatureShare)>>();

        let combined_sign = pks
            .combine_signatures(signs.iter().map(|(i, s)| (*i, s)))
            .unwrap();

        let qc = GenericQC::new(view, &hash, &combined_sign);

        parent = hash;
        prev_qc = Box::new(GenericQC::hash(&qc));

        nodes.push(node);
        qcs.push(Box::new(qc));
        // make qc
        view += 1;
    }
}
