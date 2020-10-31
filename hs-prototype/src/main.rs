//!
//! Event-driven HotStuff prototype
//! 
use std::collections::HashMap; 
use std::cell::Cell; 
use std::time::Duration;

use tokio::{
    prelude::*, 
    sync::{
        mpsc, 
    },
    runtime::Runtime, 
};

mod utils; 
use utils::threshold_sign_kit;
mod basic; 
use basic::*;


#[derive(Debug)]
pub enum MsgToSM{
    // go into next view and new leader
    ViewChange(ViewNumber, ReplicaID), 
    // Proposal from leader
    Proposal(ViewNumber, Box<TreeNode>),
    // Recv vote from replica
    Vote(ViewNumber, Box<NodeHash>),
    // 
    NewLeaf(Box<QCHash>, Vec<Cmd>, mpsc::Sender<TreeNode>), 
}

#[derive(Debug, Clone)]
pub struct SofeState{
    view: ViewNumber, 
}

#[derive(Debug, Clone)]
pub struct HardState{
    last_exec_node: NodeHash, 
}

// Tell Core works.
#[derive(Debug, Clone)]
pub enum Ready{
    State, 
    // New proposal to be voted.
    NewProposal(Box<TreeNode>), 
    // Response from a replica for a certain proposal. 
    VoteReply(Box<NodeHash>, Sign), 
    // Update notification to Pacemaker. 
    UpdateQCHigh(Box<GenericQC>), 
}

pub struct StateMachine{
    pks: PK, 
    sk: SK, 
    tree: HashMap<NodeHash, TreeNode>, 
    qc_map: HashMap<QCHash, GenericQC>, 
    // record last vote set
    vote: HashMap<ReplicaID, Sign>,  

    view: ViewNumber, 
    last_executed: NodeHash, 
    last_node: NodeHash, 
    locked_node: TreeNode, 
    leader: Option<ReplicaID>, 

    msg_in: mpsc::Receiver<MsgToSM>, 
    msg_out: mpsc::Sender<Ready>, 
    commit_ch: mpsc::Sender<NodeHash>,
}
  
impl StateMachine{

    pub fn new(sks: SK, pks: PK) -> (StateMachine, mpsc::Receiver<Ready>, mpsc::Sender<MsgToSM>){
        let buf_sz = 16; 
        let (in_sender, in_recv) = mpsc::channel(buf_sz);
        let (out_sender, out_recv) = mpsc::channel(buf_sz);
        let (commit_s, commit_r) = mpsc::channel(64);
        let f = 1; 
        let n = 3 * f + 1;

        // tree
        let genesis_node = TreeNode::genesis(); 
        let mut tree = HashMap::with_capacity(16); 
        tree.insert(TreeNode::hash(&genesis_node), genesis_node); 

        let mut sm = StateMachine{
            pks: pks.clone(), 
            sk: sks, 
            tree: tree,
            qc_map: HashMap::new(), 
            vote: HashMap::new(),
            msg_in: in_recv, 
            msg_out: out_sender, 
            commit_ch: commit_s, 
            last_executed: NodeHash::genesis(), 
            last_node: NodeHash::genesis(),
            locked_node: TreeNode::genesis(), 
            view: 0, 
            leader: None, 
        };

        (sm, out_recv, in_sender)
    }

    pub async fn step(&mut self, msg: MsgToSM){
        match msg{
            MsgToSM::NewLeaf(qc, cmds, mut os) => {
                let (node, hash) = TreeNode::node_and_hash(cmds, self.view, self.last_node.clone(), *qc);
                self.last_node = *hash;
                os.send(node).await.unwrap();
            },
            MsgToSM::ViewChange(view, rid) => {
                self.view_change(view, rid);
            }, 
            // new proposal from leader
            MsgToSM::Proposal(view, node) => {
                self.on_recv_proposal(&node).await;
            },
            MsgToSM::Vote(view, b_node_hash) => {
                self.on_recv_vote();
            },
            _ => {}
        }
    }
 
    // is a <-- .. <- b
    // becare of ddos.
    fn is_successor(&self, a: &TreeNode, b:&TreeNode) -> bool{
        let mut ref_node = b; 
        while a.height > ref_node.height{
            if let Some(parent) = self.tree.get(&ref_node.parent){
                ref_node = parent; 
            }else{
                return false; 
            }
        }
        TreeNode::hash(a) == TreeNode::hash(ref_node)
    }

    fn safe_node(&self, node: &TreeNode, qc: &GenericQC) -> bool{
        self.is_successor(&self.locked_node, node) || 
        qc.view > self.locked_node.height
    }

    fn view_change(&mut self, view: ViewNumber, rid: ReplicaID){
        if self.view > view{
            return; 
        }
        self.view = view;
        self.leader = Some(rid); 
        self.vote.clear();          // 重置
    }

    /// 接受来自leader的proposal. 忽略view <= self.view的prooposal
    async fn on_recv_proposal(&mut self, node: &TreeNode){
        let h = TreeNode::hash(node);
        let justify = self.qc_map.get(&node.justify).unwrap(); 
        if node.height > self.view && self.safe_node(&node, justify){
            self.view = node.height;
            self.add_new_leaf(&h, node.clone()).await;
            self.sign_and_respond(node, &h).await;
        }
        self.update_node(&h).await;
    }

    // vote
    async fn on_recv_vote(&mut self){
        unimplemented!() 
    }

    async fn sign_and_respond(&mut self, node: &TreeNode, h: &NodeHash){
        // sign and then send response to leader. 
        let sign = mix_sign(&self.sk, node, node.height);
        self.msg_out.send(
            Ready::VoteReply(Box::new(h.clone()), sign)
        ).await.unwrap();
    }

    async fn add_new_leaf(&mut self, h:&NodeHash, node: TreeNode){
        self.last_node = h.clone();
        self.tree.insert(self.last_node.clone(), node);
    }

    // nodehash -> Generic
    fn get_qc(&self, h:&NodeHash) -> Option<GenericQC>{
        match self.tree.get(h){
            None => None, 
            Some(node) => {
                match self.qc_map.get(&node.justify){
                    None => None, 
                    Some(qc) => Some(qc.clone()), 
                }
            }
        }
    }

    // b1 <- b2 <- b3 <- b
    fn get_three_chain(&self, h: &NodeHash) -> Vec<TreeNode>{
        let mut res = Vec::with_capacity(3); 
        if let Some(b) = self.tree.get(h){
            if let Some(q3) = self.qc_map.get(&b.justify){
                let b3 = &q3.node; 
                res.push(b3.clone()); 
                if let Some(q2) = self.qc_map.get(&b3.justify){
                    let b2 = &q2.node; 
                    res.push(b2.clone()); 
                    if let Some(q1) = self.qc_map.get(&b3.justify){
                        let b1 = &q1.node; 
                        res.push(b1.clone()); 
                    }
                }
            }
        }
        res
    }
    
    fn is_continuous_three_chain(&self, candidates: &Vec<TreeNode>) -> bool{
        if candidates.len() != 3{
            return false; 
        }
        let mut hash = TreeNode::hash(&candidates[1]);
        if candidates[0].parent != hash{
            return false; 
        }
        hash = TreeNode::hash(&candidates[2]);
        candidates[1].parent == hash
    }

    async fn update_node(&mut self, h: &NodeHash){
        if let Some(qc) = self.get_qc(h){
            self.update_qc_high(&qc).await;
        }

        // get b3, b2, b1 
        let prevs = self.get_three_chain(h); 
        // update b2 -> locked
        if let Some(b2) = prevs.get(1){
            if b2.height > self.locked_node.height{
                self.locked_node = b2.clone();
            }
        }

        // if b3, b2, b1 chained --> commit b
        if prevs.len() == 3 && self.is_continuous_three_chain(&prevs){
            let h = TreeNode::hash(&prevs.last().unwrap());
            self.on_commit(&h).await;
            self.last_executed = h; 
        }
    }

    // Tell pacemaker to update qc_high.
    async fn update_qc_high(&mut self, qc: &GenericQC){
        self.msg_out.send(
            Ready::UpdateQCHigh(Box::new(qc.clone()))
        ).await.unwrap();
    }

    /// Commit all nodes preceeding node h(inclueded) using accumulative ack. 
    async fn on_commit(&mut self, h: &NodeHash){
        let to_executed = self.tree.get(h).unwrap(); 
        let last_executed = self.tree.get(&self.last_executed).unwrap();
        if last_executed.height < to_executed.height{
            self.last_executed = h.clone(); 
            self.commit_ch.send(h.clone()).await.unwrap();
        }
    }
}

pub async fn run(mut sm: StateMachine, mut stop_ch: mpsc::Receiver<()>){
    let mut break_flag = false; 
    loop {
        tokio::select! {
            Some(msg) = sm.msg_in.recv() => {
                sm.step(msg).await;
            }, 
            Some(()) = stop_ch.recv() => {
                break_flag = true;
            }
        }; 

        if break_flag{
            break;
        }
    }
    // drop msg_out .
    drop(sm); 
    println!("step down");
}

#[cfg(test)]
mod sm_test{
    use super::*; 

    fn assert_continuous(nodes: &Vec<TreeNode>){
        let mut hash = None; 
        for node in nodes{
            let tmp = TreeNode::hash(node); 
            if hash.is_some() && hash.unwrap() != tmp{
                println!("{:?} - {:?}", node.height, node.cmds);
                assert!(false);
            }
            hash = Some(node.parent.clone()); 
        }
    }

    #[test]
    fn test_down(){
        let (sks, pk, _) = threshold_sign_kit(4, 2);
        let (sm, _,_) = StateMachine::new(sks.secret_key_share(1), pk);

        let mut rt = Runtime::new().unwrap();
        rt.block_on(
            async {
                let (mut stop_sender, stop_ch) = mpsc::channel(1);
                stop_sender.send(()).await.unwrap();
                tokio::spawn(run(sm, stop_ch)).await.unwrap();
            }
        );
    }
    
    #[test ]
    //#[ignore]
    fn test_new_leaf(){
        let (sks, pk, _) = threshold_sign_kit(4, 2);
        let (sm, _, mut msg_sender) = StateMachine::new(sks.secret_key_share(1), pk);
        let (mut stop_sender, stop_ch) = mpsc::channel(1);
        let inputs = vec![
            vec!["Hello, world!".to_string()], 
            vec!["LGTM".to_string()],
        ];

        Runtime::new().unwrap().block_on(
            async {
                // sender 
                tokio::spawn(run(sm, stop_ch));
                let mut os_sender = vec![];
                let mut os_recv = vec![];
                for _ in 0..inputs.len(){
                    let (s, r) = mpsc::channel::<TreeNode>(1);
                    os_sender.push(s);
                    os_recv.push(r);
                }

                tokio::spawn(
                    async move {
                        let mut view = 0; 
                        for (cmds, sender) in inputs.into_iter().zip(os_sender){
                            // one node per view. 
                            view += 1;
                            msg_sender.send(MsgToSM::ViewChange(view, format!("node-1"))).await.unwrap();
                            msg_sender.send(MsgToSM::NewLeaf(Box::new(QCHash::genesis()), cmds, sender)).await.unwrap();
                        }
                        // drop after checker receive TreeNode.
                        tokio::time::delay_for(Duration::from_secs(1)).await;
                        stop_sender.send(()).await.unwrap();
                    }
                );

                tokio::spawn(
                    async move{
                        let mut outputs = vec![];
                        for mut recv in os_recv{
                            outputs.push(recv.recv().await.unwrap());
                        }
                        outputs.sort_by(|a, b| a.height.cmp(&b.height));
                        outputs.reverse();
                        assert_continuous(&outputs);
                    }
                ).await.unwrap();

            }
        );
    }
    #[test]
    fn test_commit(){

    }
}

#[test]
#[ignore = "for dev only"]
fn ds_size(){
    macro_rules! size{
        ($x:ty, $($y:ty),+) => {
            size!($x); 
            size!($($y),+)
        };
        ($x:ty) => {
            println!("size of {:12} = {:4} Bytes.", stringify!($x), std::mem::size_of::<$x>()); 
        }; 
    }

    size!(
        PK, SK, Sign, CombinedSign, ReplicaID, ViewNumber, NodeHash, GenericQC, TreeNode
    ); 
}

pub trait Storage{
}

pub trait Network{}

pub struct HSCore<M, P, S:Storage, N:Network>{
    // stateMachine
    sm: M, 
    pm: P, 
    st: S, 
    nc: N, 
}

fn main() {
}