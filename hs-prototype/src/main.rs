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
use actix_web::{get, web, App, HttpServer, Responder};
use serde::{Serialize, Deserialize};
use log::{info, error}; 
use simplelog::*; 

mod utils; 
mod basic;
mod traits; 
use utils::*; 
use basic::*; 
use traits::*; 

#[derive(Clone, Debug)]
pub enum InternalMsg{
    RequestSnapshot(Sender<Box<Snapshot>>),
    Sign(Sender<(Context, Arc<TreeNode>, Box<Sign>)>), 
    RecvProposal(Context, Arc<TreeNode>), 
    Propose(Context, Arc<TreeNode>, Sender<ViewNumber>), 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot{
    view: ViewNumber, 
    tick: u64, 
    node_executed: NodeHash, 
    qc_high: QCHash, 
}

const PROP_COMMITTED: &str = "committed"; 
const PROP_LOCKED: &str = "locked"; 
const PROP_QUEUING: &str = "queuing"; 

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalState{
    view: ViewNumber, 
    // committed, locked, 
    state: &'static str, 
    // node_hash
}

struct NetKit{
    // peer_conf: HashMap<ReplicaID, String>, 
    self_id: String, 
    self_port: u16, 
    sender: Sender<InternalMsg>, 
    recvr: Receiver<InternalMsg>, 
}

struct InternalSender{
    sender: Sender<InternalMsg>, 
}

impl NetKit{
    fn new(ip: String, port: u16) -> (Self, Sender<InternalMsg>, Receiver<InternalMsg>){
        let (nk_sender, recvr) = channel(64);
        let (sender, nk_recvr) = channel(64); 

        let nk = NetKit{
            self_id: ip, 
            self_port: port, 
            sender: nk_sender, 
            recvr: nk_recvr, 
        };

        (nk, sender, recvr)

    }

    async fn status(data: web::Data<InternalSender>) -> String{
        let (ss_sender, mut ss_recvr) = channel(1); 
        let mut sender = data.sender.clone(); 
        if let Err(e) = sender.send(
            InternalMsg::RequestSnapshot(
                ss_sender
            )
        ).await{
            error!("{}", e); 
            return format!("internal error"); 
        }

        match ss_recvr.recv().await{
            None => {
                error!("sender dropped early"); 
                format!("internal error")
            }, 
            Some(ss) => 
                serde_json::to_string(ss.as_ref()).unwrap(), 
        }

    }

}

struct Machine{
    node_pool: HashMap<NodeHash, Arc<TreeNode>>, 
    qc_map: HashMap<QCHash, Arc<GenericQC>>, 
    leaf: Arc<TreeNode>,
    leaf_high: ViewNumber, 
    qc_high: Arc<GenericQC>, 
    view: ViewNumber, 
    // viewnumber of last voted treenode.
    vheight: ViewNumber, 
    commit_height: ViewNumber, 

    tick_ch: Receiver<(u64, u64)>, 
    tick: u64, 
    deadline: u64, 

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

    net_in: Receiver<InternalMsg>, 
    net_out: Sender<InternalMsg>, 
}

impl Machine{
    fn new(
        self_id: &String, 
        sign_id: u32,   
        pks: PK, 
        sks: SK,
        tick_ch: Receiver<(u64, u64)>, 
        net_ch: (Sender<InternalMsg>, Receiver<InternalMsg>), 
        peer_conf: HashMap<String, String>, ) -> Machine
    {
        let node = TreeNode::genesis(); 
        let first_qc = GenericQC::genesis(0, &node); 
        let mut qc_map = HashMap::with_capacity(4); 
        qc_map.insert(GenericQC::hash(&first_qc), Arc::new(first_qc.clone())); 

        let (net_out, net_in) = net_ch; 
        Machine{
            node_pool: HashMap::new(), 
            qc_map,
            leaf: Arc::new(node.clone()),
            leaf_high: 0, 
            qc_high: Arc::new(first_qc), 
            view: 0, 
            vheight: 0, 
            commit_height: 0, 
            tick_ch, 
            tick: 0, 
            deadline: 1, 
            b_executed: Arc::new(node.clone()), 
            b_locked: Arc::new(node.clone()), 
            peer_conf, 
            self_id: self_id.clone(),  
            leader_id: None, 
            sign_id: sign_id, 
            pks, 
            sks, 
            voting_set: HashMap::new(), 
            decided: false,  
            net_in, 
            net_out, 
        }
    }

    fn id(&self) -> (String, ViewNumber, u64, u64){
        (self.self_id.clone(), self.view, self.tick, self.deadline)
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
    
    fn find_three_chain(&self, node_hash: &NodeHash) -> Vec<Arc<TreeNode>> {
        let mut chain = Vec::with_capacity(3); 
        // b''
        let mut ptr = node_hash.clone(); 
        if let Some(qc) = self.find_qc_by_justify(&ptr){
            chain.push(self.node_pool.get(&qc.node).unwrap().clone()); 
            ptr = qc.node.clone(); 
        }
        // b'
        if let Some(qc) = self.find_qc_by_justify(&ptr){
            chain.push(self.node_pool.get(&qc.node).unwrap().clone()); 
            ptr = qc.node.clone(); 
        }
        // b
        if let Some(qc) = self.find_qc_by_justify(&ptr){
            chain.push(self.node_pool.get(&qc.node).unwrap().clone()); 
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

    fn update_qc_high(&mut self, qc_node: &TreeNode, qc_high: &GenericQC) {
        let h = GenericQC::hash(qc_high); 
        // let qc_node = self.find_node_by_qc(&qc_high).unwrap(); 
        let prev_node = self.find_node_by_qc(&h).unwrap(); 
        if prev_node.height < qc_node.height{
            self.qc_high = Arc::new(qc_high.clone()); 
            self.leaf = Arc::new(qc_node.clone()); 
            self.leaf_high = qc_node.height; 
        }
    }

    fn is_conflicting(&self, mut a:&TreeNode, mut b: &TreeNode) -> bool {
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

        node.height == b.height && node.cmds == b.cmds
    }

    fn get_node(&mut self, node_hash: &NodeHash) -> Option<Arc<TreeNode>>{
        self.node_pool
        .get(node_hash)
        .and_then(|node| Some(node.clone()))
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

    fn update_nodes(&mut self, node: &TreeNode) {
        let b = node; 
        let h = TreeNode::hash(node); 
        let chain = self.find_three_chain(&h); 
        let b_lock = self.get_locked_node(); 

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

    fn safe_node(&mut self, node: &TreeNode, qc: &GenericQC) -> bool {
        !self.is_conflicting(node, self.b_locked.as_ref()) ||
        qc.view > self.b_locked.height
    }
}

impl Timer for Machine{
    fn reset_timer(&mut self){
        self.tick; 
        self.deadline = 0; 
    }

    fn tick(&mut self, delta: u64){
        self.tick += delta; 
    }

    fn deadline(&self) -> u64{
        self.deadline
    }
    
    fn update_deadline(&mut self, deadline: u64){
        self.deadline = deadline;
    }

    fn touch_deadline(&self) -> bool{
        self.tick >= self.deadline
    }
}

#[async_trait::async_trait]
impl Network for Machine{
    async fn propose(&mut self, node: &TreeNode) {
        /*
        self.net_out.send(
            InternalMsg::Propose(
                Context{
                    view: self.get_view(), 
                    from: self.self_id.clone(), 
                }, 
                Arc::new(node.clone()), 
            )
        ).await.unwrap(); 
        */
    }

    async fn accept_proposal(&mut self, ctx: &Context, node: &TreeNode, sign: &SignKit) {
        unimplemented!()
    }

    async fn new_leader(&mut self, ctx: &Context, leader: &ReplicaID) {
        unimplemented!()
    }
}

impl Pacemaker for Machine{
    fn leader_election(&mut self) {
        unimplemented!()
    }

    fn view_change(&mut self) {
        self.reset(); 
        self.increase_view(self.view + 1); 
        info!("{:?} view change", self.id());
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

impl HotStuff for Machine{

    fn is_leader(&self) -> bool {
        self.leader_id
        .as_ref()
        .map_or(
            false, 
            |leader| leader == &self.self_id
        )
    }

    // start prposal
    fn on_beat(&mut self, cmds: &Vec<Cmd>) {
        let prev_leaf = self.get_leaf(); 
        let parent = TreeNode::hash(prev_leaf.as_ref());
        let justify = GenericQC::hash(self.get_qc_high().as_ref());
        let (node, _) = TreeNode::node_and_hash(
            cmds, 
            self.view, 
            &parent, 
            &justify,
        ); 

        // update leaf & statemachine. 
        self.append_new_node(node.as_ref());
        self.update_leaf(node.as_ref());

        // broadcast 
        self.propose(node.as_ref());
    }

     // Return or ignore if self is not the leader. 
    fn on_recv_vote(&mut self, ctx: &Context, node:&TreeNode, sign: &SignKit){
        if self.decided || self.add_vote(ctx, sign){
            return ;
        }
        // vote at most once. 
        if self.vote_set_size() > self.threshold(){
            //self.compute_combined_sign(); 
            let combined_sign = self.combine_partial_sign(); 

            // TODO: leaf as node <=> no new proposal 
            let node_hash = TreeNode::hash(node); 
            let qc = GenericQC{
                view: self.view, 
                node: node_hash, 
                combined_sign: Some(*combined_sign), 
            };
            self.update_qc_high(node, &qc);
            self.decided = true; 
        }
    }  

    // Return immediately if self is not the leader. 
    fn on_recv_proposal(&mut self, ctx: &Context, node: &TreeNode){
        let node_hash = TreeNode::hash(node); 
        if let Some(justify) = self.find_qc_by_justify(&node_hash){
            if node.height > self.vheight && self.safe_node(node, justify.as_ref()){
                self.vheight = node.height; 
                // send reply
                let kit = SignKit{
                    sign: *self.sign(node), 
                    sign_id: self.sign_id(), 
                };
                self.accept_proposal(ctx, node, &kit); 
            }

        }
    }
}

fn default_timer(mut tick_sender: Sender<(u64, u64)>, lifetime: u64, step: u64){
    tokio::spawn(
        async move {
            let step = step; 
            let mut tick = 0; 
            // 5 tick per view
            let mut deadline = tick + step; 
            loop{
                delay_for(Duration::from_secs(1)).await; 
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
                // process tick
                tick = self.tick_ch.recv() => {
                    match tick{
                        Some((new_tick, deadline)) => {
                            self.process_tick(new_tick, deadline); 
                        }, 
                        None => {
                            down = true; 
                        }, 
                    }
                }, 
                Some(request) = self.net_in.recv() => {
                    self.process_net_request(request).await; 
                }, 
            }

            if down{
                break; 
            }
        }
        info!("{:?} down", self.id()); 
    }

    fn process_tick(&mut self, new_tick:u64, deadline: u64){
        self.tick(new_tick - self.tick); 
        self.update_deadline(deadline); 
        // info!("{:?} tick", self.id());
        if self.touch_deadline(){
            info!("{:?} timeout", self.id());
            self.view_change(); 
            self.update_deadline(self.tick + 1); 
        }
    }

    async fn process_net_request(&mut self, req: InternalMsg){
        match req{
            InternalMsg::RequestSnapshot(mut sender) => {
                let ss = Box::new(
                    Snapshot{
                        view: self.view, 
                        tick: self.tick, 
                        node_executed: TreeNode::hash(self.get_last_executed().as_ref()), 
                        qc_high: GenericQC::hash(self.get_qc_high().as_ref()), 
                    }
                ); 

                // self.net_out.send(InternalMsg::ReplySnapshot(ss)).await.unwrap();
                sender.send(ss).await.unwrap();
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
        let (k, _) = kv; 
        let (sign_id, sks) = sk_conf; 
        let peers = backup.clone(); 
        let pks = pks.clone(); 
        let (net_kit, net_out, net_in) = NetKit::new(format!("localhost"), 8000+sign_id as u16); 
        let mc = Machine::new(
            &k, 
            sign_id as u32, 
            pks, 
            sks, 
            tick_recvr, 
            (net_out, net_in),
            peers,
        );
        let addr = mc.get_addr(&mc.self_id).unwrap().clone(); 
        
        // timer 
        timers.push(tick_sender); 
        handlers.push(handler); 
        mcs.push((mc, stop)); 
        nks.push(net_kit); 
    }

    let tokio_rt_handler = std::thread::spawn(
        move || {
            tokio::runtime::Runtime::new().unwrap().block_on(
                async move {
                    for (mc, tick_sender) in mcs.into_iter().zip(timers){
                        
                        default_timer(tick_sender, 100, 5);
                        spawn(
                            async move {
                                let (mut mc, mut stop) = mc;
                                mc.run().await; 
                                stop.send(()).await.unwrap(); 
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

    let actix_web_rt_handler = std::thread::spawn(
        move || {
            actix_web::rt::Runtime::new().unwrap().block_on(
                run_proxy(nks)
            ) 
        }
    );

    tokio_rt_handler.join().unwrap(); 
}

async fn run_proxy(nks: Vec<NetKit>){
    for net_kit in nks{
        let local = tokio::task::LocalSet::new(); 
        let sys = actix_web::rt::System::run_in_tokio("hotstuff proxy", &local); 
        let sender = net_kit.sender.clone(); 
        info!("netkit is listening"); 
        let addr = format!("{}:{}", net_kit.self_id.clone(), net_kit.self_port);

        HttpServer::new(move ||{
            App::new()
            .data(
                // shared state
                InternalSender{
                    sender: sender.clone(), 
                }
            )
            .service(
                web::scope("/hotstuff").route("status", web::get().to(NetKit::status))
            )
        })
        .bind(addr).unwrap()
        .run()
        .await.unwrap(); 
        sys.await.unwrap();
    }
}
