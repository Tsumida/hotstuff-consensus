//! State machine matains safety of hotstuff node. 

use std::collections::{
    HashMap, 
};

use tokio::sync::mpsc;
use crate::basic::*;

use log::{
    info,
};

#[derive(Debug)]
pub enum MsgToSM{
    // go to next view
    ViewChange(ViewNumber, ReplicaID), 

    // As leader, Propose a new leaf and return it to Pacemaker through given channel.
    // QCHash points to qc_high in Pacemaker.
    Propose(Box<QCHash>, Vec<Cmd>, mpsc::Sender<Box<TreeNode>>), 

    // QCHigh from replicas. 
    RecvQCHigh(Box<GenericQC>), 

    // Receive proposal from leader
    RecvProposal(Box<TreeNode>),

    // Recv vote from replica.
    Vote(u32, Box<NodeHash>, Box<Sign>),

    // Get Status
    StateSnapshot(mpsc::Sender<Box<(SoftState, HardState)>>), 
}

#[derive(Debug, Clone)]
pub struct SoftState{
    view: ViewNumber, 
    nodes: HashMap<NodeHash, TreeNode>, 
    qc_map: HashMap<QCHash, GenericQC>, 
}

#[derive(Debug, Clone)]
pub struct HardState{
    vheight: ViewNumber, 
    last_exec_node: NodeHash, 
}

// Tell Core works.
#[derive(Debug, Clone)]
pub enum Ready{
    State(Box<(SoftState, HardState)>), 
    // As replica, sign a Proposal.
    VoteReply(Box<NodeHash>, Sign), 
    // As Leader or replica, Update notification to Pacemaker. 
    UpdateQCHigh(Box<GenericQC>), 
}

pub struct StateMachine{
    sm_id: String, 

    total: usize, 

    pks: PK, 
    sk: SK, 
    tree: HashMap<NodeHash, TreeNode>, 
    qc_map: HashMap<QCHash, GenericQC>, 
    // record partial signatures for node with height = vheight. 
    vote: HashMap<usize, Sign>, 
    // 
    view: ViewNumber, 
    // height of the leaf. Avoid vote twice or vote for outdated Propose.
    vheight: ViewNumber, 
    // The parent of next leaf. 
    leaf: NodeHash, 
    // Indicating whether the leaf reach consensus or not, 
    // helping avoiding meaningless threshold signature computing.
    reach_consensus: bool, 
    last_executed: NodeHash, 
    
    locked_node: TreeNode, 
    leader: Option<ReplicaID>, 

    // Receive event for state transfering. 
    msg_in: mpsc::Receiver<MsgToSM>, 
    // Send notifications about what other component to do. 
    msg_out: mpsc::Sender<Ready>, 
    commit_ch: mpsc::Sender<Box<NodeHash>>,
}
  
impl StateMachine{
    /// Create new state machine. 
    pub fn new(sm_id: String, sks: SK, pks: PK) 
         -> (StateMachine, mpsc::Receiver<Ready>, mpsc::Sender<MsgToSM>, mpsc::Receiver<Box<NodeHash>>)
    {
        let buf_sz = 16; 
        let (in_sender, in_recv) = mpsc::channel(buf_sz);
        let (out_sender, out_recv) = mpsc::channel(buf_sz);
        let (commit_s, commit_r) = mpsc::channel(64);
        let f = 1; 
        let n = 3 * f + 1;

        let genesis_node = TreeNode::genesis(); 
        let genesis_qc = GenericQC::genesis(0, &genesis_node);
        // qc_map
        let mut qc_map = HashMap::with_capacity(16); 
        qc_map.insert(QCHash::genesis(), genesis_qc);
        // tree
        let mut tree = HashMap::with_capacity(16); 
        tree.insert(TreeNode::hash(&genesis_node), genesis_node); 

        let sm = StateMachine{
            sm_id, 
            total: n, 
            pks: pks.clone(), 
            sk: sks, 
            tree: tree,
            qc_map, 
            vote: HashMap::with_capacity(n),
            msg_in: in_recv, 
            msg_out: out_sender, 
            commit_ch: commit_s, 
            leaf: NodeHash::genesis(), 
            reach_consensus: true, 
            last_executed: NodeHash::genesis(), 
            locked_node: TreeNode::genesis(), 
            view: 0, 
            vheight: 0, 
            leader: None, 
        };

        (sm, out_recv, in_sender, commit_r)
    }

    /// Run state machine. Stop it by dropping the msg_in channel.
    /// # Example
    /// ```
    /// let f = 1; 
    /// let n = 3 * f + 1;
    /// let (sks, pks, sk) = utils::threshold_sign_kit(n, 2*f); 
    /// let (mut sm, ready_ch, msg_ch) = StateMachine::new(
    ///    format!("node-1"), 
    ///    sks.secret_key_share(0), 
    ///    pks.clone());
    /// 
    /// tokio::spawn(
    ///     async move {
    ///         sm.run().await;
    ///     }
    /// ); 
    /// drop(msg_ch); // state machine stops asynchronously. 
    /// ``` 
    ///
    pub async fn run(&mut self){
        let mut break_flag = false; 
        loop {
            tokio::select!{
                r = self.msg_in.recv() => {
                    match r{
                        Some(msg) => self.step(msg).await, 
                        None => break_flag = true, 
                    }
                }, 
            }
    
            if break_flag{
                break;
            }
        }
        // drop msg_out .
        info!("state machine {} step down", self.sm_id);
    }

    /// step() consumes MsgToSM and then change internal state. 
    async fn step(&mut self, msg: MsgToSM){
        match msg{
            MsgToSM::Propose(qc, cmds, mut os) => {
                let (node, hash) = TreeNode::node_and_hash(cmds, self.view, self.leaf.clone(), *qc);
                self.leaf = hash.as_ref().clone();
                self.add_new_leaf(hash.as_ref(), node.as_ref().clone()).await;
                os.send(node).await.unwrap();
            },
            MsgToSM::ViewChange(view, rid) => {
                self.view_change(view, rid);
            }, 
            MsgToSM::RecvProposal(node) => {
                self.on_recv_proposal(&node).await;
            },
            MsgToSM::Vote(from, b_node_hash, sign) => {
                self.on_recv_vote(from, b_node_hash, sign).await;
            },
            MsgToSM::StateSnapshot(mut sender) => {
                let bx = self.status(); 
                sender.send(bx).await.unwrap(); 
            },
            _ => {}
        }
    }

    /// Take status snapshot. 
    fn status(&self) -> Box<(SoftState, HardState)>{
        Box::new(
            (
                SoftState{
                    view: self.view, 
                    nodes: self.tree.clone(), 
                    qc_map: self.qc_map.clone(), 
                }, 
                HardState{
                    last_exec_node: self.last_executed.clone(), 
                    vheight: self.vheight, 
                }
            )
        )
    }
 
    /// TODO: need unit test
    fn is_successor(&self, a: &TreeNode, b:&TreeNode) -> bool{
        if a.height <= b.height{
            return false; 
        }
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

    // Change view and reset status. 
    fn view_change(&mut self, view: ViewNumber, rid: ReplicaID){
        if self.view > view{
            return; 
        }
        self.view = view;
        self.leader = Some(rid); 
        self.vote.clear();         
        self.reach_consensus = false; 
    }
    
    // nodehash -> Generic
    /*
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
    }*/

    /// Get node.justify 
    fn get_qc_h(&self, h:&NodeHash) -> Option<Box<(QCHash, GenericQC)>>{
        match self.tree.get(h){
            None => None, 
            Some(node) => {
                match self.qc_map.get_key_value(&node.justify){
                    None => None, 
                    Some((k, v)) => Some(Box::new((k.clone(), v.clone()))), 
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
                    if let Some(q1) = self.qc_map.get(&b2.justify){
                        let b1 = &q1.node; 
                        res.push(b1.clone()); 
                    }
                }
            }
        }
        res
    }
    
    /// c[0] -> c[1] -> c[2]
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

    #[inline(always)]
    fn threshold(&self) -> usize{
        // overflow if self.total == 0
        ((self.total - 1) << 1) / 3
    }

    /// As replica, recv new proposal from leader.
    /// Update qc_high == proposal.node.justify
    async fn on_recv_proposal(&mut self, node: &TreeNode){
        let h = TreeNode::hash(node);
        let justify = self.qc_map.get(&node.justify).unwrap(); 
        if node.height > self.vheight && self.safe_node(&node, justify){
            self.vheight = node.height;
            self.add_new_leaf(&h, node.clone()).await;
            self.sign_and_respond(node, &h).await;
        }
        self.update_node(&h).await;
    }

    // As leader
    async fn on_recv_vote(&mut self, from: u32, hash: Box<NodeHash>, sign: Box<Sign>){
        // duplicate vote. 
        if self.reach_consensus || self.vote.insert(from as usize, *sign).is_some(){
            return; 
        }
        // lead to at most n/3 + 1 updateQCHigh
        if self.vote.len() > self.threshold(){
            let iter = self.vote
                .iter()
                .map(|(i, node)| (i, node))
                .collect::<Vec<_>>(); 
            match self.pks.combine_signatures(iter){
                Ok(combined_sign) => {
                    let qc = GenericQC::new(
                        self.view,
                        self.tree.get(&hash).unwrap().clone(),
                        combined_sign,
                    ); 
                    self.update_qc_high(&qc).await; 
                    self.qc_map.insert(GenericQC::hash(&qc), qc);
                    // avoiding second computing. 
                    self.reach_consensus = true; 
                }, 
                Err(e) => panic!(e), 
            }
        }
    }

    async fn sign_and_respond(&mut self, node: &TreeNode, h: &NodeHash){
        // sign and then send response to leader. 
        let sign = crate::basic::sign(&self.sk, node, node.height);
        self.msg_out.send(
            Ready::VoteReply(Box::new(h.clone()), sign)
        ).await.unwrap();
    }

    async fn add_new_leaf(&mut self, h:&NodeHash, node: TreeNode){
        self.leaf = h.clone();
        self.tree.insert(self.leaf.clone(), node);
    }

    // Update state
    async fn update_node(&mut self, h: &NodeHash){
        if let Some(qc) = self.get_qc_h(h){
            let (k, v) = qc.as_ref(); 
            self.update_qc_high(v).await;
            self.qc_map.insert(k.clone(), v.clone());
        }

        // get b3, b2, b1 
        let prevs = self.get_three_chain(h); 
        // update b2 -> locked
        if let Some(b2) = prevs.get(1){
            if b2.height > self.locked_node.height{
                self.locked_node = b2.clone();
            }
        }

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
            self.commit_ch.send(Box::new(h.clone())).await.unwrap();
        }
    }
}


 
#[cfg(test)]
mod sm_test{
    use super::*; 
    
    use std::time::Duration;
    use crate::utils::threshold_sign_kit;

    #[test]
    /// 1. All replicas send their qc_high to leader.
    /// 2. Leader propose a new node and get signs from all replicas (including leader itself). 
    /// 3. Leader computes the combined signature , which will be part of a GenericQC qc. 
    /// 4. Leader commit nodes preceeding a certain node b that forms a three-chain. 
    /// 5. Leader send the qc within Ready::UpdateQCHigh for next proposal. 
    fn test_propose(){
        let n = 4; 
        let leader = 2usize;  //leader
        let (sks, pk, _) = threshold_sign_kit(n, 2);
        let (mut sm, mut msg_out, mut msg_sender, _) = StateMachine::new(format!("node-2"), sks.secret_key_share(leader), pk);
        let inputs = vec![
            vec!["Hello, world!".to_string()], 
            vec!["miao!".to_string()],
        ];

        let genesis_qc = GenericQC::genesis(0, &TreeNode::genesis()); 
        let genesis_qc_hash = GenericQC::hash(&genesis_qc);
        let (p_send, mut p_recv) = mpsc::channel(1); 

        tokio::runtime::Runtime::new().unwrap().block_on(
            async {
                // Run state machine
                tokio::spawn(
                    async move {
                        sm.run().await;
                    }
                );
                // leaf sender (for sm) and receiver (for external component). 
                let mut os_sender = vec![];
                let mut os_recv = vec![];
                for _ in 0..inputs.len(){
                    let (s, r) = mpsc::channel::<Box<TreeNode>>(1);
                    os_sender.push(s);
                    os_recv.push(r);
                }
                // checker. 
                tokio::spawn(
                    async move {
                        let mut view = 0; 
                        // Replica propose 2 proposal in view 1, 2 but fail to reach consensus. 
                        for (cmds, sender) in inputs.into_iter().zip(os_sender){
                            view += 1;
                            msg_sender.send(MsgToSM::ViewChange(view, format!("node-2"))).await.unwrap();
                            msg_sender.send(MsgToSM::Propose(Box::new(QCHash::genesis()), cmds, sender)).await.unwrap();
                        }

                        // Now Replica 2 become leader in view 3. 
                        view += 1; 
                        msg_sender.send(MsgToSM::ViewChange(view, format!("node-2"))).await.unwrap(); 
                       
                        // Pacemaker functions. 
                        // Replicas send their qc_high in NewView Msg. 
                        
                        // Pacemaker will take qc with largest viewNumber(height) from replicas as qc_high. 
                        // Let Leader 2 forms new proposal and waits for votes. 
                        msg_sender.send(
                            MsgToSM::Propose(Box::new(genesis_qc_hash), vec!["wangwang".to_string()], p_send)
                        ).await.unwrap(); 

                        // Consume new nodes. 
                        let mut outputs = vec![Box::new(TreeNode::genesis())];
                        for mut recv in os_recv{
                            outputs.push(recv.recv().await.unwrap());
                        }

                        // lastest Proposal 
                        let proposal = p_recv.recv().await.unwrap(); 
                        let hash = TreeNode::hash(proposal.as_ref()); 
                        assert!(proposal.height == 3); 
                        // compute partial signatures from replica (leader included)
                        let signs = (0..4u32).map(
                            |i| {
                                sign(
                                    &sks.secret_key_share(i as usize ), 
                                    proposal.as_ref(), 
                                    proposal.height)
                            }
                        );

                        for (i, s) in signs.enumerate(){
                            msg_sender.send(
                                MsgToSM::Vote(i as u32, Box::new(hash.clone()), Box::new(s))
                            ).await.unwrap(); 
                        }
                        
                        match msg_out.recv().await{
                            Some(Ready::UpdateQCHigh(qc)) => {
                                assert!(qc.node.cmds == vec!["wangwang".to_string()]);
                            }, 
                            _ => assert!(false), 
                        }

                        // drop after checker receive TreeNode.
                        tokio::time::delay_for(Duration::from_secs(1)).await;
                        // sm step down. 
                        drop(msg_sender);
                        while let Some(ready) = msg_out.recv().await{   
                            println!("left ready: {:?}", ready);
                        }
                    }
                ).await.unwrap();
            }
        );
        println!("test_commit done"); 
    }

    #[test]
    #[ignore = "unimpl"]
    /// Branch switching.
    fn test_branch_switching(){
    }


}