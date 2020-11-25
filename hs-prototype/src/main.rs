//! Demo
use std::{
    collections::{
        HashMap, 
    }, 
    time::{
        Duration
    }, 
    sync::{
        Arc,
    }, 
};

use tokio::sync::mpsc::{Sender, Receiver, channel}; 
use tokio::time::delay_for; 
use tokio::spawn; 
use actix_web::{get, web, App, HttpServer, HttpResponse};
use serde::{Serialize, Deserialize};
use log::{info, error, debug}; 
use simplelog::*; 

mod utils; 
mod basic;
mod traits; 
use utils::*; 
use basic::*; 
use traits::*; 

#[derive(Clone, Debug)]
pub enum InternalMsg{

    // restful api
    RequestSnapshot(Sender<(Context, Box<Snapshot>)>),

    // As replica, recv proposal from leader.  -> on_recv_proposal
    RecvProposal(Context, Arc<TreeNode>, Arc<GenericQC>, Sender<(Context, Box<TreeNode>, Box<SignKit>)>), 

    // As leader, recv sign(ACK) from other replicas. 
    RecvSign(Context, Arc<TreeNode>, Arc<SignKit>), 

    // TODO:
    // Pacemaker related. As replica, update qc_high.
    // Note that the replica may not receive the proposal until receive this msg. 
    RecvNewViewMsg(Context, Arc<TreeNode>, Arc<GenericQC>), 

    // TODO: 
    // pacemaker -> statemachine, and then send new view msg. 
    // leader_id, new_view, 
    ViewChange(ReplicaID, u64, Sender<(Arc<TreeNode>, Arc<GenericQC>)>), 

    // As leader, broadcast proposal. 
    Propose(Context, Arc<TreeNode>, Arc<GenericQC>), 

    // New transaction from client. 
    NewTx(Cmd),

    // TODO: duplicate, remove it.
    // NewLeader
    NewLeader(Context, ReplicaID), 
    // tell http server to stop; 
    Down, 
}

impl InternalMsg{
    pub fn from_rpc_response(resp: RpcResponse) -> Option<Self>{
        if resp.status != RpcStatus::Accept || resp.node.is_none() || resp.sign.is_none(){
            return None; 
        }
        Some(
            InternalMsg::RecvSign(
                resp.ctx, 
                Arc::new(*resp.node.unwrap()), 
                Arc::new(*resp.sign.unwrap()), 
            )
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot{
    view: ViewNumber, 
    leader: Option<ReplicaID>, 
    threshold: usize, 
    // approximately
    node_num: usize, 
    leaf: Box<NodeHash>, 
    qc_map: Option<Box<Vec<(QCHash, GenericQC)>>>, 
    // node_executed: Box<NodeHash>, 
    qc_high: Box<QCHash>,

}

const PROP_COMMITTED: &str = "committed"; 
const PROP_LOCKED: &str = "locked"; 
const PROP_QUEUING: &str = "queuing"; 

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RpcType{
    NewProposal, 
    NewView, 
    NewLeader, 
    Status, 
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RpcStatus{
    Ok, 
    InvalidUsage,
    InternalError, 

    // new proposal related 
    Accept, 
    Reject, 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewTransaction{
    pub client_id: String, 
    pub cmd: String, 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest{
    pub ctx: Context, 
    pub msg_type: RpcType, 
    pub leader: Option<ReplicaID>, 
    pub prop: Option<Box<TreeNode>>, 
    pub qc: Option<Box<GenericQC>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse{
    pub ctx: Context, 
    pub status: RpcStatus, 
    pub msg_type: RpcType, 
    pub node: Option<Box<TreeNode>>,
    pub sign: Option<Box<SignKit>>, 
    pub snapshot: Option<Box<Snapshot>>, 
    pub qc: Option<Box<GenericQC>>, 
}

impl RpcResponse{
    pub fn accept(ctx: &Context, node: &TreeNode, sign: &SignKit) -> Self{
        RpcResponse{
            ctx: ctx.clone(), 
            status: RpcStatus::Accept, 
            msg_type: RpcType::NewProposal, 
            node: Some(Box::new(node.clone())),
            sign: Some(Box::new(sign.clone())), 
            snapshot: None, 
            qc: None, 
        }
    }

    pub fn reject(ctx: &Context) -> Self{
        RpcResponse{
            ctx: ctx.clone(), 
            status: RpcStatus::Reject, 
            msg_type: RpcType::NewProposal, 
            node: None,
            sign: None, 
            snapshot: None, 
            qc: None, 
        }
    }

    pub fn internal_err(ctx: &Context, msg_type: RpcType) -> Self{
        RpcResponse{
            ctx: ctx.clone(), 
            status: RpcStatus::InternalError, 
            msg_type, 
            node: None,
            snapshot: None, 
            sign: None, 
            qc: None, 
        }
    }

    pub fn invalid_usage(ctx: &Context, msg_type: RpcType) -> Self{
        RpcResponse{
            ctx: ctx.clone(), 
            status: RpcStatus::InvalidUsage, 
            msg_type, 
            node: None,
            snapshot: None, 
            sign: None, 
            qc: None, 
        }
    }

    pub fn status(ctx: &Context, ss: Option<Box<Snapshot>>) -> Self{
        RpcResponse{
            ctx: ctx.clone(), 
            status: RpcStatus::Ok, 
            msg_type: RpcType::Status, 
            snapshot: ss.and_then(|ss| Some(Box::new(*ss))), 
            node: None,
            sign: None, 
            qc: None, 
        }
    }

    pub fn new_view(ctx: &Context, node: &TreeNode, qc: &GenericQC) -> Self{
        RpcResponse{
            ctx: ctx.clone(), 
            status: RpcStatus::Ok,
            msg_type: RpcType::NewView, 
            snapshot: None, 
            node: Some(Box::new(node.clone())),
            sign: None, 
            qc: Some(Box::new(qc.clone())), 
        }
    }

    pub fn ok(ctx: &Context, msg_type: RpcType) -> Self{
        RpcResponse{
            ctx: ctx.clone(), 
            status: RpcStatus::Ok,
            msg_type, 
            snapshot: None, 
            node: None,
            sign: None, 
            qc: None,  
        }
    }
}

struct HotStuffProxy{
    peer_conf: HashMap<ReplicaID, String>, 
    self_id: String, 
    self_addr: String, 
    // to hotstuff
    sender: Sender<InternalMsg>, 
    // from hotstuff
    recvr: Receiver<InternalMsg>, 
}

struct SharedState{
    //replica_id: ReplicaID, 
    //from: ReplicaID, 
    ctx: Context, 
    sender: Sender<InternalMsg>, 
}

impl HotStuffProxy{
    fn new(self_id:String, self_addr:String, peer_conf: HashMap<ReplicaID, String>) -> (Self, Sender<InternalMsg>, Receiver<InternalMsg>){
        let (nk_sender, recvr) = channel(64);
        let (sender, nk_recvr) = channel(64); 

        let nk = HotStuffProxy{
            self_id, 
            self_addr, 
            peer_conf, 
            sender: nk_sender, 
            recvr: nk_recvr, 
        };

        (nk, sender, recvr)
    }

    async fn status(data: web::Data<SharedState>) -> HttpResponse{
        let (ss_sender, mut ss_recvr) = channel(1); 
        let mut sender = data.sender.clone(); 
        let ctx = &data.ctx; 
        if let Err(e) = sender.send(
            InternalMsg::RequestSnapshot(
                ss_sender
            )
        ).await{
            error!("{}", e); 
            return HttpResponse::Ok().json(
                RpcResponse::status(ctx, None)
            );
        }

        let resp = match ss_recvr.recv().await{
            None => {
                error!("sender dropped early"); 
                RpcResponse::status(ctx, None)
            }, 
            Some((ctx, ss)) => RpcResponse::status(&ctx, Some(ss))
                
        }; 

        HttpResponse::Ok().json(resp)
    }

    // #[post()]
    async fn recv_new_proposal(sd: web::Data<SharedState>, data: web::Json<RpcRequest>) -> HttpResponse{
        let mut sender = sd.sender.clone(); 
        let ctx = &data.ctx; 
        // TODO: ugly
        let node = Arc::new(data.prop.as_ref().unwrap().as_ref().clone());
        let justify = Arc::new(data.qc.as_ref().unwrap().as_ref().clone());
        let (sk_sender, mut sk_recvr) = channel(1); 
        
        if let Err(e) = sender.send(InternalMsg::RecvProposal(ctx.clone(), node, justify, sk_sender,)).await{
            error!("{}", e); 
            return HttpResponse::InternalServerError().json(
                RpcResponse::internal_err(ctx, RpcType::NewProposal)
            );
        }
        HttpResponse::Ok().json(
            match sk_recvr.recv().await{
                None => {
                    info!("{} reject proposal", &sd.ctx.from);
                    RpcResponse::reject(ctx)
                }, 
                Some((ctx, node, sign)) => {
                    info!("{} accept proposal", &sd.ctx.from); 
                    RpcResponse::accept(&ctx, node.as_ref(), sign.as_ref())
                }, 
            }
        )
    }

    async fn on_recv_new_view(sender: web::Data<SharedState>, data: web::Json<RpcRequest>) -> HttpResponse{
        match (&data.qc, &data.prop){
            // TODO: replace OK(). 
            (Some(ref qc), Some(ref node)) => {
                debug!("server recv new view"); 
                let mut sender = sender.sender.clone(); 
                let _ = sender.send(
                    InternalMsg::RecvNewViewMsg(data.ctx.clone(), Arc::new(node.as_ref().clone()), Arc::new(qc.as_ref().clone()))
                ).await; 
                HttpResponse::Ok().json(RpcResponse::ok(&data.ctx, RpcType::NewView))
            }, 
            _ => {
                HttpResponse::Ok().json(RpcResponse::invalid_usage(&data.ctx, RpcType::NewView))
            }
        }
    }

    /// Recv new leader information about leader.
    async fn new_leader(sender: web::Data<SharedState>, data: web::Json<RpcRequest>) -> HttpResponse{
        let mut sender = sender.sender.clone(); 
        if let Some(leader) = &data.leader{
            sender.send(InternalMsg::NewLeader(data.ctx.clone(), leader.clone())).await.unwrap(); 
            HttpResponse::Ok().json(RpcResponse::ok(&data.ctx, RpcType::NewLeader))
        }else{
            HttpResponse::Ok().json(RpcResponse::invalid_usage(&data.ctx, RpcType::NewLeader))
        }
    }

    async fn new_tx(sender: web::Data<SharedState>, data: web::Json<NewTransaction>) -> HttpResponse{
        let mut sender = sender.sender.clone(); 
        let resp = match sender.send(InternalMsg::NewTx(data.cmd.clone())).await{
            Err(e) => {
                error!("send new tx failed: {}", e); 
                "internal error"
            }, 
            Ok(_) => "ok, tx is queuing", 
        }; 
        HttpResponse::Ok().json(resp)
    }

}

struct Machine{
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

impl Machine{
    fn new(
        self_id: &String, 
        sign_id: u32,   
        pks: PK, 
        sks: SK,
        net_ch: (Sender<InternalMsg>, Receiver<InternalMsg>), 
        peer_conf: HashMap<String, String>, ) -> Machine
    {
        let node = TreeNode::genesis();
        let mut node_pool = HashMap::with_capacity(4);
        node_pool.insert(TreeNode::hash(&node), Arc::new(node.clone()));
        let first_qc = GenericQC::genesis(0, &node); 
        let mut qc_map = HashMap::with_capacity(4); 
        qc_map.insert(GenericQC::hash(&first_qc), Arc::new(first_qc.clone())); 

        let (step_out, step_in) = net_ch; 
        Machine{
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

    fn id(&self) -> (String, ViewNumber){
        (self.self_id.clone(), self.view)
    }
}

impl MemPool for Machine{
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
    fn find_qc_by_justify(&self, node_hash: &NodeHash) -> Option<Arc<GenericQC>>{
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
        if let Some(qc3) = self.qc_map.get(&node.justify){
            if let Some(b3) = self.node_pool.get(&qc3.node){
                chain.push(b3.clone());
                if let Some(qc2) = self.qc_map.get(&b3.justify){
                    if let Some(b2) = self.node_pool.get(&qc2.node){
                        chain.push(b2.clone()); 
                        if let Some(qc1) = self.qc_map.get(&b2.justify){
                            if let Some(b1) = self.node_pool.get(&qc1.node){
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
        if self.leaf_high < new_leaf.height{
            self.leaf_high = new_leaf.height; 
            self.leaf = Arc::new(new_leaf.clone()); 
            self.node_pool.insert(TreeNode::hash(new_leaf), self.leaf.clone());
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
        if let Some(qc_node) = self.find_node_by_qc(&GenericQC::hash(qc_high.as_ref())){
            let pred = new_qc_node.height > qc_node.height; 
            if pred{
                self.qc_high = Arc::new(new_qc_high.clone());
                self.leaf_high = new_qc_node.height; 
                self.leaf = Arc::new(new_qc_node.clone());
                debug!("update qc-high");
            }
        }
    }

    fn is_conflicting(&self, a:&TreeNode, b: &TreeNode) -> bool {
        let (a, b) = if a.height >= b.height{
            (a, b)
        }else{
            (b, a)
        };

        // a.height >= b.height
        let mut node = a; 
        while node.height > b.height{
            if let Some(prev) = self.node_pool.get(&node.parent){
                node = prev.as_ref(); 
            }else{
                break; 
            }
        }

        !(node.height == b.height && node.cmds == b.cmds)
    }

    fn get_node(&self, node_hash: &NodeHash) -> Option<Arc<TreeNode>>{
        self.node_pool.get(node_hash).and_then(|node| Some(node.clone()))
    }

    fn get_qc(&self, qc_hash: &QCHash) -> Option<Arc<GenericQC>>{
        self.qc_map.get(qc_hash).and_then(|qc| Some(qc.clone()))
    }

    fn get_locked_node(&self) -> Arc<TreeNode>{
        self.b_locked.clone()
    }

    fn update_locked_node(&mut self, node: &TreeNode){
        self.b_locked = Arc::new(node.clone()); 
    }

    fn get_last_executed(&self) -> Arc<TreeNode>{
        self.b_executed.clone()
    }

    fn update_last_executed_node(&mut self, node: &TreeNode){
        self.b_executed = Arc::new(node.clone()); 
    }

    fn get_view(&self) -> ViewNumber{
        self.view
    }

    fn increase_view(&mut self, new_view: ViewNumber){
        self.view = ViewNumber::max(self.view, new_view); 
    }

    fn is_continues_three_chain(&self, chain: &Vec<impl AsRef<TreeNode>>) -> bool{
        if chain.len() != 3{
            return false; 
        }

        assert!(chain.len() == 3); 
        let b_3 = chain.get(0).unwrap().as_ref(); 
        let b_2 = chain.get(1).unwrap().as_ref();
        let b = chain.get(2).unwrap().as_ref(); 

        &b_3.parent == &TreeNode::hash(b_3) &&
        &b_2.parent == &TreeNode::hash(b)
    }

    fn reset(&mut self) {
        self.voting_set.clear(); 
        self.decided = false; 
    }

    fn add_vote(&mut self, ctx: &Context, sign: &SignKit) -> bool{
        self.voting_set.insert(ctx.from.clone(), sign.clone()).is_none()
    }

    fn vote_set_size(&self) -> usize{
        self.voting_set.len()
    }
}

impl StateMachine for Machine{
    /// As replica, recv proposal from leader. 
    /// Note that
    fn update_nodes(&mut self, node: &TreeNode) {

        let chain = self.find_three_chain(node); 
        let b_lock = self.get_locked_node(); 

        // debug!("find chain with {} nodes", chain.len());

        if let Some(b_3) = chain.get(0){
            let b3_qc = self.get_qc(&node.justify).unwrap();
            self.update_qc_high(b_3.as_ref(), b3_qc.as_ref());
        }

        if let Some(b_2) = chain.get(1){
            if b_2.height > b_lock.height{
                self.update_locked_node(node); 
            }
        }

        if chain.len() == 3 && self.is_continues_three_chain(&chain){
            self.on_commit(chain.last().unwrap()); 
        }
    }

    fn on_commit(&mut self, node: &TreeNode) {
        let b_exec = self.get_locked_node();
        if b_exec.height < node.height{
            if let Some(parent) = self.get_node(&node.parent).clone(){
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

impl SysConf for Machine{
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

impl Crypto for Machine{
    fn sign_id(&self) -> SignID {
        self.sign_id
    }

    fn sign(&self, node:&TreeNode) -> Box<Sign> {
        let buf = node.to_be_bytes();
        let s = self.sks.sign(&buf); 

        Box::new(s)
    }

    fn combine_partial_sign(&self) -> Box<CombinedSign> {
        // wrapper
        let tmp = self.voting_set
        .values()
        .map(|kit| (kit.sign_id as usize, &kit.sign))
        .collect::<Vec<_>>(); 

        Box::new(self.pks.combine_signatures(tmp).unwrap())
    }
}

#[async_trait::async_trait]
impl HotStuff for Machine{

    fn is_leader(&self) -> bool {
        self.leader_id
        .as_ref()
        .map_or(
            false, 
            |leader| leader == &self.self_id
        )
    }

    async fn propose(&mut self, node: &TreeNode, qc: Arc<GenericQC>) {
        self.step_out.send(
            InternalMsg::Propose(
                Context{
                    view: self.get_view(), 
                    from: self.self_id.clone(), 
                }, 
                Arc::new(node.clone()), 
                qc, 
            )
        ).await.unwrap(); 
    }

    // start prposal
    async fn on_beat(&mut self, cmds: &Vec<Cmd>) {
        info!("{} beats", self.self_id); 
        let prev_leaf = self.get_leaf(); 
        let parent = TreeNode::hash(prev_leaf.as_ref());
        let justify = GenericQC::hash(self.get_qc_high().as_ref());
        let (node, _) = TreeNode::node_and_hash(
            cmds, 
            self.view, 
            &parent, 
            &justify,
        ); 
        
        // debug!("new proposal info: view{}\n\tparent: {:?}\n\tjustify: {:?}", node.height, &parent, &justify);

        // update leaf & statemachine. 
        self.update_leaf(node.as_ref());

        // broadcast 
        self.propose(node.as_ref(), self.get_qc_high()).await;
    }

    // TODO: seperate hashing. 
    // Return or ignore if self is not the leader. 
    async fn on_recv_vote(&mut self, ctx: &Context, prop:&TreeNode, sign: &SignKit){
        if self.decided || !self.add_vote(ctx, sign){
            return ;
        }
        info!("recv partial sign from {}", &ctx.from);
        // vote at most once. 
        if self.vote_set_size() > self.threshold(){
            //self.compute_combined_sign(); 
            let combined_sign = self.combine_partial_sign(); 
            // TODO: leaf as prop <=> no new proposal 
            let prop_hash = TreeNode::hash(prop); 
            let qc = GenericQC{
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
    async fn on_recv_proposal(&mut self, ctx: &Context, prop: &TreeNode, justify: &GenericQC, mut sender: Sender<(Context, Box<TreeNode>, Box<SignKit>)>){
        if let Some(ref leader) = self.leader_id{
            if leader != &ctx.from{
                debug!("{} reject prop due to diff leader.", &self.self_id); 
                return; 
            }
        }else{
            debug!("{} reject prop due to diff leader.", &self.self_id); 
            return; 
        }

        self.append_new_qc(justify);
        if let Some(prev_node) = self.find_node_by_qc(&prop.justify){
            // debug!("{} found qc.", &self.self_id); 
            if prop.height > self.vheight && self.safe_node(prop, prev_node.as_ref()){
                self.vheight = prop.height; 
                let kit = SignKit{
                    sign: *self.sign(prop), 
                    sign_id: self.sign_id(), 
                };
                sender.send((
                    Context{view: self.view, from: self.self_id.clone()},
                    Box::new(prop.clone()), 
                    Box::new(kit))).await.unwrap();
                self.append_new_node(&prop);
            }
        }
        self.update_nodes(prop);
    }
}

fn default_timer(mut tick_sender: Sender<(u64, u64)>, lifetime: u64, step: u64){
    tokio::spawn(
        async move {
            let step = step; 
            let mut tick = 0; 
            let mut deadline = tick + step; 
            loop{
                delay_for(Duration::from_secs(4)).await; 
                if tick >= lifetime || tick_sender.send((tick, deadline)).await.is_err(){
                    break; 
                }
                tick += 1; 
                if tick > deadline{
                    deadline += step; 
                }
            }
            drop(tick_sender);
        }
    );
}

impl Machine{
    async fn run(&mut self) {
        let mut down = false; 
        loop{
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
            if down { break; }
        }
        info!("{:?} down", self.id()); 
    }

    async fn process_internal_msg(&mut self, req: InternalMsg){
        match req{
            InternalMsg::RequestSnapshot(mut sender) => {
                let ss = Box::new(
                    Snapshot{
                        view: self.view, 
                        leader: self.leader_id.clone(),
                        threshold: self.threshold(), 
                        node_num: self.node_pool.len(), 
                        leaf: Box::new(TreeNode::hash(self.get_last_executed().as_ref())), 
                        qc_high: Box::new(GenericQC::hash(self.get_qc_high().as_ref())), 
                        qc_map: None, 
                    }
                ); 
                sender.send((
                    Context{from: self.self_id.clone(), view: self.view}, 
                    ss, 
                )).await.unwrap();
            }, 
            InternalMsg::RecvProposal(ctx, proposal, justify, sender) => {
                self.on_recv_proposal(&ctx, proposal.as_ref(), justify.as_ref(), sender).await; 
            }, 
            InternalMsg::RecvSign(ctx,  node, sign) =>{
                self.on_recv_vote(&ctx, node.as_ref(), sign.as_ref()).await; 
            }, 
            InternalMsg::NewTx(cmd) => {
                // TODO: use pool
                let v = vec![cmd]; 
                self.on_beat(&v).await;
            }, 
            InternalMsg::RecvNewViewMsg(_, _, qc_high) => {
                // TDOO: 
                // note: recv largest qc_high
                if let Some(qc_node) = self.get_node(&qc_high.node){
                    self.update_qc_high(qc_node.as_ref(), qc_high.as_ref());
                }
            },
            // TODO: remove
            InternalMsg::NewLeader(ctx, leader) => {
                if ctx.view >= self.view && self.leader_id.is_none(){
                    self.leader_id = Some(leader); 
                }
            },
            InternalMsg::ViewChange(leader, view, _) => {
                if view > self.view{
                    self.view = view; 
                    self.leader_id = Some(leader); 
                    self.reset();
                    info!("view change {}", self.view);
                }
            }, 
            _ => error!("recv invalid msg"), 
        }
    }
}

fn main(){
    let _ = CombinedLogger::init(
        vec![
            TermLogger::new(LevelFilter::Info, Config::default(),TerminalMode::Mixed), 
        ], 
    );
    
    info!("hotstuff demo"); 
    info!("hotstuff node ({}) Bytes", std::mem::size_of::<Machine>());

    let f = 1; 
    let n = 3 * f + 1;
    let peer_config = (0..n)
    .map(|i| (format!("node-{}", i), format!("localhost:{}", 8000+i)))
    .collect::<HashMap<String, String>>(); 

    let (_, pks, sk) = threshold_sign_kit(n, 2*f); 

    // run machines. 
    let mut mcs = vec![]; 
    let mut handlers = vec![]; 
    let mut nks = vec![]; 
    let mut timers = vec![];
    // peer_conf dos not live enough. 
    let backup = peer_config.clone(); 
    for (kv, sk_conf) in peer_config.into_iter().zip(sk){
        // TODO: unbounded channel
        let (mut stop, handler) = channel(1); 
        let (tick_sender, tick_recvr) = channel(16); 
        let (k, v) = kv; 
        let (sign_id, sks) = sk_conf; 
        let peers = backup.clone(); 
        let pks = pks.clone(); 
        let (hs_proxy, to_net, from_net) = HotStuffProxy::new(k.clone(), v, backup.clone()); 
        let mc = Machine::new(
            &k, 
            sign_id as u32, 
            pks, 
            sks, 
            (to_net.clone(), from_net),
            peers,
        );

        let pm = Pacemaker::new(
            backup.clone(), 
            tick_recvr, 
            hs_proxy.sender.clone(), 
            to_net.clone(), 
            Some(format!("node-0")),
        );
        
        // timer 
        timers.push(tick_sender); 
        handlers.push(handler); 
        mcs.push((mc, stop, pm)); 
        nks.push(hs_proxy); 
    }

    let tokio_rt_handler = std::thread::spawn(
        move || {
            tokio::runtime::Runtime::new().unwrap().block_on(
                async move {
                    for (kit, tick_sender) in mcs.into_iter().zip(timers){
                        default_timer(tick_sender, 1000, 5);
                        let (mut mc, mut stop, mut pm) = kit;
                        spawn(
                            async move {
                                mc.run().await; 
                                stop.send(()).await.unwrap(); 
                            }
                        ); 
                        spawn(
                            async move {
                                pm.run().await;
                            }
                        ); 
                    }
                    for mut h in handlers{
                        h.recv().await.unwrap(); 
                    }
                }
            );
        }
    );

    for hs_proxy in nks{
        std::thread::spawn(
            move || {
                actix_web::rt::Runtime::new().unwrap().block_on(
                    run_server(hs_proxy)
                )
            }
        );
    }

    tokio_rt_handler.join().unwrap(); 
}

struct Pacemaker{
    view: u64, 
    tick: u64, 
    deadline: u64, 
    tick_ch: Receiver<(u64, u64)>, 
    to_sm: Sender<InternalMsg>, 
    to_net: Sender<InternalMsg>, 
    peer_conf: HashMap<ReplicaID, String>, 
    leader: Option<ReplicaID>, 
}

impl Pacemaker{

    pub fn new(
        peer_conf: HashMap<ReplicaID, String>, 
        tick_ch: Receiver<(u64, u64)>, 
        to_sm: Sender<InternalMsg>, 
        to_net: Sender<InternalMsg>,
        leader: Option<ReplicaID>, 
    )   -> Self
    {
        Pacemaker{
            view: 1, 
            tick: 0, 
            deadline: 1 << 10, 
            tick_ch, 
            to_sm,
            to_net, 
            peer_conf, 
            leader, 
        }
    }

    async fn run(&mut self){
        let mut down = false;  
        loop{
            tokio::select! {
                Some((new_tick, deadline)) = self.tick_ch.recv() => {
                    self.tick = u64::max(new_tick, self.tick); 
                    self.deadline = deadline; 
                    if self.touch_deadline(){
                        self.view += 1; 
                        self.view_change().await;
                    }
                }, 
            }
            if down{
                break; 
            }
        }
    }

    fn reset_timer(&mut self){
        self.tick; 
        self.deadline = 0; 
    }

    fn touch_deadline(&self) -> bool{
        self.tick >= self.deadline
    }

    // TODO
    async fn view_change(&mut self){
        let (sender, mut recvr) = channel(1); 
        self.to_sm.send(
            InternalMsg::ViewChange(
                self.leader.as_ref().unwrap().clone(), 
                self.view, 
                sender, 
            )
        ).await.unwrap(); 
    }
}

async fn broadcastor(peer_conf: &HashMap<ReplicaID, String>, resp_sender: Sender<InternalMsg>) -> (tokio::sync::broadcast::Sender<()>, tokio::sync::broadcast::Sender<RpcRequest>){
    let (fan_out_s, _) = tokio::sync::broadcast::channel::<RpcRequest>(1); 
    let (down_s, _) = tokio::sync::broadcast::channel::<()>(1);
    
    for (k, v) in peer_conf.iter(){
        let (_, addr) = (k.clone(), v.clone()); 
        let mut listener = fan_out_s.subscribe();
        let mut down = down_s.subscribe(); 
        let mut resp_sender = resp_sender.clone(); 
        actix_web::rt::spawn(
            async move{
                let mut exit = false; 
                let url = format!("http://{}/hotstuff/new-proposal", addr); 
                loop {
                    tokio::select! {
                        Ok(ref prop) = listener.recv() => {
                            // debug!("posting {}", &url); 
                            match actix_web::client::Client::default().post(&url).send_json(prop).await{
                                Ok(mut resp) =>{
                                    let rpc_resp: RpcResponse = resp.json().await.unwrap();
                                    if let Some(msg) = InternalMsg::from_rpc_response(rpc_resp){
                                        resp_sender.send(msg).await.unwrap(); 
                                    }
                                }, 
                                Err(e) => {
                                    error!("error {}", e);
                                }
                            }
                        }, 
                        Ok(_) = down.recv() =>{
                            exit = false; 
                        }
                    }
                    if exit{
                        break; 
                    }
                }
            }
        )
    }


    (down_s, fan_out_s)
}

async fn run_server(hs_proxy: HotStuffProxy){
    let local = tokio::task::LocalSet::new(); 
    let sys = actix_web::rt::System::run_in_tokio("hotstuff proxy", &local);
    
    let sender_server = hs_proxy.sender.clone(); 
    info!("HotStuffProxy is listening"); 
    let addr = hs_proxy.self_addr.clone();    
    let self_id = hs_proxy.self_id.clone();
    let (mut svr_sender, mut svr_recvr) = channel(1); 

    // http server 
    // TODO: tidy
    actix_web::rt::spawn(
        async move {
            info!("http server running");
            
            let server = HttpServer::new(move ||{
                App::new()
                .data(
                    SharedState{
                        // TODO: update ctx 
                        ctx: Context{
                            from: self_id.clone(), 
                            view: 0, 
                        }, 
                        sender: sender_server.clone(), 
                    }
                )
                .service(
                    web::scope("/hotstuff")
                        .route("status", web::get().to(HotStuffProxy::status))
                        // new proposal from leader
                        .route("new-proposal", web::post().to(HotStuffProxy::recv_new_proposal))
                        // recv new view from other replicas. 
                        .route("new-view", web::post().to(HotStuffProxy::on_recv_new_view))
                        // who's new leader
                        .route("new-leader", web::post().to(HotStuffProxy::new_leader))
                        // tx from client to commit 
                        .route("new-tx", web::post().to(HotStuffProxy::new_tx))
                )
            })
            .bind(addr).unwrap()
            .run(); 

            svr_sender.send(server).await.unwrap(); 
            
        }
    );

    // TODO: tidy
    // proxy loop 
    actix_web::rt::spawn(
        async move {
            let svr = svr_recvr.recv().await.unwrap(); 
            let mut hs_proxy = hs_proxy; 
            let mut quit = false; 
            // let mut sender_client = sender_client; 
            let (fan_out_handler, fan_out) = broadcastor(&hs_proxy.peer_conf, hs_proxy.sender.clone()).await;

            loop {
                tokio::select! {
                    Some(msg) = hs_proxy.recvr.recv() => {
                        match msg{
                            InternalMsg::Propose(ctx, node, qc) => { 
                                info!("proposing"); 
                                fan_out.send(
                                    RpcRequest{
                                        ctx, 
                                        msg_type: RpcType::NewProposal, 
                                        leader: None, 
                                        prop: Some(Box::new(node.as_ref().clone())), 
                                        qc: Some(Box::new(qc.as_ref().clone())), 
                                    }
                                ).unwrap();
                            }, 
                            InternalMsg::Down => {
                                quit = true; 
                                let _ = fan_out_handler.send(());
                            }
                            _ => error!("invalid msg"), 
                        }
                    }, 
                }
                if quit{
                    break;
                }
            }
            svr.stop(false).await; 
        }
    ); 
    sys.await.unwrap();
}
