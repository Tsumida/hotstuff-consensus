pub mod basic;
pub mod msg;
pub mod traits;
use basic::*;
use msg::Context;
use msg::*;
use traits::*;

use std::{collections::HashMap, sync::Arc};

use log::{debug, error, info};
use tokio::sync::mpsc::{Receiver, Sender};

pub struct Machine {
    // TODO: consider remove this queue
    node_pool: HashMap<NodeHash, Arc<TreeNode>>,
    qc_map: HashMap<QCHash, Arc<GenericQC>>,
    leaf: Arc<TreeNode>,
    leaf_high: ViewNumber,
    qc_high: Arc<GenericQC>,
    view: ViewNumber,
    // viewnumber of last voted treenode.
    vheight: ViewNumber,
    commit_height: ViewNumber,

    b_executed: Arc<TreeNode>,
    b_locked: Arc<TreeNode>,

    peer_conf: HashMap<ReplicaID, String>,
    self_id: ReplicaID,
    leader_id: Option<ReplicaID>,

    sign_id: SignID,
    pks: PK,
    sks: SK,

    voting_set: HashMap<ReplicaID, SignKit>,
    decided: bool,

    step_in: Receiver<InternalMsg>,
    step_out: Sender<InternalMsg>,
}

impl Machine {
    pub fn new(
        self_id: &String,
        sign_id: u32,
        pks: PK,
        sks: SK,
        net_ch: (Sender<InternalMsg>, Receiver<InternalMsg>),
        peer_conf: HashMap<String, String>,
    ) -> Machine {
        let node = TreeNode::genesis();
        let mut node_pool = HashMap::with_capacity(4);
        node_pool.insert(TreeNode::hash(&node), Arc::new(node.clone()));
        let first_qc = GenericQC::genesis(0, &node);
        let mut qc_map = HashMap::with_capacity(4);
        qc_map.insert(GenericQC::hash(&first_qc), Arc::new(first_qc.clone()));

        let (step_out, step_in) = net_ch;
        Machine {
            node_pool,
            qc_map,
            leaf: Arc::new(node.clone()),
            leaf_high: 0,
            qc_high: Arc::new(first_qc),
            view: 1,
            vheight: 0,
            commit_height: 0,
            b_executed: Arc::new(node.clone()),
            b_locked: Arc::new(node.clone()),
            peer_conf,
            self_id: self_id.clone(),
            // TODO
            leader_id: Some(format!("node-0")),
            sign_id: sign_id,
            pks,
            sks,
            voting_set: HashMap::new(),
            decided: false,
            step_in,
            step_out,
        }
    }

    fn id(&self) -> (String, ViewNumber) {
        (self.self_id.clone(), self.view)
    }
}

impl MemPool for Machine {
    fn append_new_node(&mut self, node: &TreeNode) {
        let h = TreeNode::hash(node);
        self.node_pool.insert(h, Arc::new(node.clone()));
    }

    fn append_new_qc(&mut self, qc: &GenericQC) {
        let h = GenericQC::hash(qc);
        self.qc_map.insert(h, Arc::new(qc.clone()));
    }

    fn find_parent(&self, node: &TreeNode) -> Option<Arc<TreeNode>> {
        self.node_pool
            .get(&node.parent)
            .and_then(|node| Some(node.clone()))
    }

    // node should be in node pool.
    fn find_qc_by_justify(&self, node_hash: &NodeHash) -> Option<Arc<GenericQC>> {
        self.node_pool
            .get(node_hash)
            .and_then(|node| self.qc_map.get(&node.justify))
            .and_then(|qc| Some(qc.clone()))
    }

    fn find_node_by_qc(&self, qc_hash: &QCHash) -> Option<Arc<TreeNode>> {
        self.qc_map
            .get(qc_hash)
            .and_then(|qc| self.node_pool.get(&qc.node))
            .and_then(|node| Some(node.clone()))
    }

    fn find_three_chain(&self, node: &TreeNode) -> Vec<Arc<TreeNode>> {
        let mut chain = Vec::with_capacity(3);
        if let Some(qc3) = self.qc_map.get(&node.justify) {
            if let Some(b3) = self.node_pool.get(&qc3.node) {
                chain.push(b3.clone());
                if let Some(qc2) = self.qc_map.get(&b3.justify) {
                    if let Some(b2) = self.node_pool.get(&qc2.node) {
                        chain.push(b2.clone());
                        if let Some(qc1) = self.qc_map.get(&b2.justify) {
                            if let Some(b1) = self.node_pool.get(&qc1.node) {
                                chain.push(b1.clone());
                            }
                        }
                    }
                }
            }
        }
        chain
    }

    fn update_leaf(&mut self, new_leaf: &TreeNode) {
        if self.leaf_high < new_leaf.height {
            self.leaf_high = new_leaf.height;
            self.leaf = Arc::new(new_leaf.clone());
            self.node_pool
                .insert(TreeNode::hash(new_leaf), self.leaf.clone());
        }
    }

    fn get_leaf(&self) -> Arc<TreeNode> {
        self.leaf.clone()
    }

    fn get_qc_high(&self) -> Arc<GenericQC> {
        self.qc_high.clone()
    }

    /// qc_high.node == qc_node.
    fn update_qc_high(&mut self, new_qc_node: &TreeNode, new_qc_high: &GenericQC) {
        let qc_high = self.get_qc_high();
        if let Some(qc_node) = self.find_node_by_qc(&GenericQC::hash(qc_high.as_ref())) {
            let pred = new_qc_node.height > qc_node.height;
            if pred {
                self.qc_high = Arc::new(new_qc_high.clone());
                self.leaf_high = new_qc_node.height;
                self.update_leaf(new_qc_node);
                debug!("update qc-high");
            }
        }
    }

    fn is_conflicting(&self, a: &TreeNode, b: &TreeNode) -> bool {
        let (a, b) = if a.height >= b.height { (a, b) } else { (b, a) };

        // a.height >= b.height
        let mut node = a;
        while node.height > b.height {
            if let Some(prev) = self.node_pool.get(&node.parent) {
                node = prev.as_ref();
            } else {
                break;
            }
        }

        !(node.height == b.height && node.cmds == b.cmds)
    }

    fn get_node(&self, node_hash: &NodeHash) -> Option<Arc<TreeNode>> {
        self.node_pool
            .get(node_hash)
            .and_then(|node| Some(node.clone()))
    }

    fn get_qc(&self, qc_hash: &QCHash) -> Option<Arc<GenericQC>> {
        self.qc_map.get(qc_hash).and_then(|qc| Some(qc.clone()))
    }

    fn get_locked_node(&self) -> Arc<TreeNode> {
        self.b_locked.clone()
    }

    fn update_locked_node(&mut self, node: &TreeNode) {
        self.b_locked = Arc::new(node.clone());
    }

    fn get_last_executed(&self) -> Arc<TreeNode> {
        self.b_executed.clone()
    }

    fn update_last_executed_node(&mut self, node: &TreeNode) {
        self.b_executed = Arc::new(node.clone());
    }

    fn get_view(&self) -> ViewNumber {
        self.view
    }

    fn increase_view(&mut self, new_view: ViewNumber) {
        self.view = ViewNumber::max(self.view, new_view);
    }

    fn is_continues_three_chain(&self, chain: &Vec<impl AsRef<TreeNode>>) -> bool {
        if chain.len() != 3 {
            return false;
        }

        assert!(chain.len() == 3);
        let b_3 = chain.get(0).unwrap().as_ref();
        let b_2 = chain.get(1).unwrap().as_ref();
        let b = chain.get(2).unwrap().as_ref();

        &b_3.parent == &TreeNode::hash(b_3) && &b_2.parent == &TreeNode::hash(b)
    }

    fn reset(&mut self) {
        self.voting_set.clear();
        self.decided = false;
    }

    fn add_vote(&mut self, ctx: &Context, sign: &SignKit) -> bool {
        self.voting_set
            .insert(ctx.from.clone(), sign.clone())
            .is_none()
    }

    fn vote_set_size(&self) -> usize {
        self.voting_set.len()
    }
}

impl StateMachine for Machine {
    /// As replica, recv proposal from leader.
    /// Note that
    fn update_nodes(&mut self, node: &TreeNode) {
        let chain = self.find_three_chain(node);
        let b_lock = self.get_locked_node();

        // debug!("find chain with {} nodes", chain.len());

        if let Some(b_3) = chain.get(0) {
            let b3_qc = self.get_qc(&node.justify).unwrap();
            self.update_qc_high(b_3.as_ref(), b3_qc.as_ref());
        }

        if let Some(b_2) = chain.get(1) {
            if b_2.height > b_lock.height {
                self.update_locked_node(node);
            }
        }

        if chain.len() == 3 && self.is_continues_three_chain(&chain) {
            self.on_commit(chain.last().unwrap());
        }
    }

    fn on_commit(&mut self, node: &TreeNode) {
        let b_exec = self.get_locked_node();
        if b_exec.height < node.height {
            if let Some(parent) = self.get_node(&node.parent).clone() {
                self.on_commit(parent.as_ref());
                // execute node.
            }
            self.commit_height = node.height;
        }
    }

    // TODO: unit test
    fn safe_node(&mut self, node: &TreeNode, prev_node: &TreeNode) -> bool {
        let a = !self.is_conflicting(node, self.b_locked.as_ref());
        // TODO: in paper b_new.jusitfy.node.height
        let b = prev_node.height > self.b_locked.height;
        debug!("{} safe_node() result {} - {}", &self.self_id, a, b);
        a || b
    }
}

impl SysConf for Machine {
    fn self_id(&self) -> &str {
        self.self_id.as_ref()
    }

    fn get_addr(&self, node_id: &String) -> Option<&String> {
        self.peer_conf.get(node_id)
    }

    fn threshold(&self) -> usize {
        (self.peer_conf.len() << 1) / 3
    }
}

impl Crypto for Machine {
    fn sign_id(&self) -> SignID {
        self.sign_id
    }

    fn sign(&self, node: &TreeNode) -> Box<Sign> {
        let buf = node.to_be_bytes();
        let s = self.sks.sign(&buf);

        Box::new(s)
    }

    fn combine_partial_sign(&self) -> Box<CombinedSign> {
        // wrapper
        let tmp = self
            .voting_set
            .values()
            .map(|kit| (kit.sign_id as usize, &kit.sign))
            .collect::<Vec<_>>();

        Box::new(self.pks.combine_signatures(tmp).unwrap())
    }
}

#[async_trait::async_trait]
impl HotStuff for Machine {
    fn is_leader(&self) -> bool {
        self.leader_id
            .as_ref()
            .map_or(false, |leader| leader == &self.self_id)
    }

    async fn propose(&mut self, node: &TreeNode, qc: Arc<GenericQC>) {
        self.step_out
            .send(InternalMsg::Propose(
                Context {
                    view: self.get_view(),
                    from: self.self_id.clone(),
                },
                Arc::new(node.clone()),
                qc,
            ))
            .await
            .unwrap();
    }

    // start prposal
    async fn on_beat(&mut self, cmds: &Vec<Cmd>) {
        info!("{} beats", self.self_id);
        let prev_leaf = self.get_leaf();
        let parent = TreeNode::hash(prev_leaf.as_ref());
        let justify = GenericQC::hash(self.get_qc_high().as_ref());
        let (node, _) = TreeNode::node_and_hash(cmds, self.view, &parent, &justify);
        self.update_leaf(node.as_ref());

        // broadcast
        self.propose(node.as_ref(), self.get_qc_high()).await;
    }

    // TODO: seperate hashing.
    // Return or ignore if self is not the leader.
    async fn on_recv_vote(&mut self, ctx: &Context, prop: &TreeNode, sign: &SignKit) {
        if self.decided || !self.add_vote(ctx, sign) {
            return;
        }
        info!("recv partial sign from {}", &ctx.from);
        // vote at most once.
        if self.vote_set_size() > self.threshold() {
            //self.compute_combined_sign();
            let combined_sign = self.combine_partial_sign();
            // TODO: leaf as prop <=> no new proposal
            let prop_hash = TreeNode::hash(prop);
            let qc = GenericQC {
                view: self.view,
                node: prop_hash,
                combined_sign: Some(*combined_sign),
            };
            self.update_qc_high(&prop, &qc);

            self.decided = true;
            self.append_new_qc(&qc);
            info!("qc formed");
        }
    }

    // Return immediately if self is not the leader.
    async fn on_recv_proposal(
        &mut self,
        ctx: &Context,
        prop: &TreeNode,
        justify: &GenericQC,
        mut sender: Sender<(Context, Box<TreeNode>, Box<SignKit>)>,
    ) {
        if let Some(ref leader) = self.leader_id {
            if leader != &ctx.from {
                debug!("{} reject prop due to diff leader.", &self.self_id);
                return;
            }
        } else {
            debug!("{} reject prop due to diff leader.", &self.self_id);
            return;
        }

        self.append_new_qc(justify);
        if let Some(prev_node) = self.find_node_by_qc(&prop.justify) {
            // debug!("{} found qc.", &self.self_id);
            if prop.height > self.vheight && self.safe_node(prop, prev_node.as_ref()) {
                self.vheight = prop.height;
                let kit = SignKit {
                    sign: *self.sign(prop),
                    sign_id: self.sign_id(),
                };
                sender
                    .send((
                        Context {
                            view: self.view,
                            from: self.self_id.clone(),
                        },
                        Box::new(prop.clone()),
                        Box::new(kit),
                    ))
                    .await
                    .unwrap();
                self.append_new_node(&prop);
            }
        }
        self.update_nodes(prop);
    }
}

impl Machine {
    pub async fn run(&mut self) {
        let mut down = false;
        loop {
            // TODO: do something.
            tokio::select! {
                Some(request) = self.step_in.recv() => {
                    if let InternalMsg::Down = request{
                        down = true;
                    }else{
                        self.process_internal_msg(request).await;
                    }
                },
            }
            if down {
                break;
            }
        }
        info!("{:?} down", self.id());
    }

    async fn process_internal_msg(&mut self, req: InternalMsg) {
        match req {
            InternalMsg::RequestSnapshot(mut sender) => {
                let ss = Box::new(Snapshot {
                    view: self.view,
                    leader: self.leader_id.clone(),
                    threshold: self.threshold(),
                    node_num: self.node_pool.len(),
                    leaf: base64::encode(&self.get_last_executed().as_ref().to_be_bytes()),
                    qc_high: base64::encode(&self.get_qc_high().as_ref().to_be_bytes()),
                });
                sender
                    .send((
                        Context {
                            from: self.self_id.clone(),
                            view: self.view,
                        },
                        ss,
                    ))
                    .await
                    .unwrap();
            }
            InternalMsg::RecvProposal(ctx, proposal, justify, sender) => {
                self.on_recv_proposal(&ctx, proposal.as_ref(), justify.as_ref(), sender)
                    .await;
            }
            InternalMsg::RecvSign(ctx, node, sign) => {
                self.on_recv_vote(&ctx, node.as_ref(), sign.as_ref()).await;
            }
            InternalMsg::NewTx(cmd) => {
                // TODO: use pool
                let v = vec![cmd];
                self.on_beat(&v).await;
            }
            InternalMsg::RecvNewViewMsg(_, _, qc_high) => {
                // TDOO:
                // note: recv largest qc_high
                if let Some(qc_node) = self.get_node(&qc_high.node) {
                    self.update_qc_high(qc_node.as_ref(), qc_high.as_ref());
                }
            }
            // TODO: remove
            InternalMsg::NewLeader(ctx, leader) => {
                if ctx.view >= self.view && self.leader_id.is_none() {
                    self.leader_id = Some(leader);
                }
            }
            InternalMsg::ViewChange(leader, view, mut sender) => {
                if view > self.view {
                    self.view = view;
                    self.leader_id = Some(leader);
                    self.reset();
                    info!("view change {}", self.view);
                    let _ = sender.send((self.get_leaf(), self.get_qc_high())).await;
                }
            }
            _ => error!("recv invalid msg"),
        }
    }
}
