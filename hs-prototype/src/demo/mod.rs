
use threshold_crypto::{
    SecretKeySet
};

use rand::thread_rng;
use std::collections::HashMap;
use std::sync::mpsc::{
    channel,
    Sender,
    Receiver,
};

mod mock_net;
use mock_net::*;

use crate::{
    msg::*,
    network::*,
    utils::*,
};

struct Node<N: Network<Msg<Vec<u8>>>>{
    network: N,
    index: usize,
    sk: PrivateKey,
    pk: PublicKey,
    total: usize,  
    role: NodeRole,
}

enum NodeRole{
    Leader,
    Follower,
}

impl<N: Network<Msg<Vec<u8>>>> Node<N>{
    fn run(&mut self){
        match self.role{
            NodeRole::Follower  => self.run_as_follower(),
            NodeRole::Leader    => self.run_as_leader(),
        }
    }

    fn run_as_follower(&mut self){
        // 1. recv string from leader
        // 2. sig and reply
        // 3. wait for combined sig
        // 4. verify 

        // 1. recv string from leader
        let msg = self.network.recv();
        match msg.msg_type{
            MsgType::Raw => {
                // 2. sig and reply
                let cont: String = String::from_utf8(msg.cont).unwrap();
                
                let reply: Msg<Vec<u8>> = Msg{
                    msg_type: MsgType::Raw, 
                    from: self.index, 
                    // receiver recv Vec<u8>
                    cont: serde_json::to_vec(&self.sk.sign(&cont)).unwrap(),
                };

                self.network.send(
                    msg.from, 
                    reply, 
                );

                // 3. wait for combined sig
                let msg: Msg<_> = self.network.recv();
                let sig = serde_json::from_slice(&msg.cont).unwrap();
                if self.pk.public_key().verify(&sig, &cont){
                    println!("follower-{} verified proposal: {}", self.index, &cont);
                }else{
                    println!("follwer-{} failed to verify", self.index);
                }
            }, 
            //TODO
            _ => panic!("unexpected msg type")
        }
    }

    //TODO
    fn run_as_leader(&mut self){
        // 1. make proposal
        // 2. broadcast the msg
        // 3. collect partial sigs 
        // 4. generate combined sig
        // 5. broadcast it and then exit.
        let cont: String = format!("proposal from {}", self.index);
        let msg = Msg{
            msg_type: MsgType::Raw,
            from: self.index,
            cont: cont.bytes().collect::<Vec<u8>>(),
        };

        // 2. broadcast the msg
        for i in 0..self.total{
            if i == self.index{
                continue
            }
            self.network.send(
                i,
                msg.clone(),
            );

        }
        // 3. collect 2f+1 partial sigs
        let qc_threshold = (self.total - 1) / 3; 
        let mut replies: Vec<Signature> = Vec::with_capacity(self.total);

        // exclude self
        let qc: Vec<(usize, Msg<Vec<u8>>)> = Vec::new();
        let mut cnt = 0;
        loop {
            let msg = self.network.recv();

            match msg.msg_type {
                MsgType::Raw => {
                    
                    

                }, 
                _ => {},
            }

            if cnt > qc_threshold{
                break
            }
        }

        // 4. generate combined sig
        let sigs = qc.iter().map(|q| (q.0, &q.1)).collect::<Vec<_>>();
        assert!(sigs.len() >= qc_threshold);
        /*
        let final_sig = self.pk.combine_signatures(sigs).unwrap();

        println!("ok, leader-{} generates final signature: {:?}", self.index, &final_sig);
        let reply =  Msg{
            msg_type: MsgType::Raw, 
            from: self.index, 
            cont: serde_json::to_vec(&final_sig).unwrap(),
        };

        // 5. broadcast it and then exit.
        for i in 0..self.total{
            if i == self.index{
                continue
            }
            self.network.send(i, reply.clone());
        }*/
    }


}



pub fn demo() {
    // DKG
    let n: usize = 4;
    let t: usize = 2;
    let sks = SecretKeySet::random(t, &mut thread_rng());
    let pks = sks.public_keys();
    let mut ths = Vec::with_capacity(n);
    // node -> mock_net
    let (mn_sender, mn_recver) = channel();
    let mut mock_net = MockNetwork{
        out: HashMap::with_capacity(n),
        mailbox: mn_recver,
    };

    // leader election
    let leader = 2;
    // create 4 node
    for i in 0..n{
        let partial_sk = sks.secret_key_share(i);
        let pks = sks.public_keys();
        let mn_s = mn_sender.clone();

        // mock_net -> node
        let (sender, recver) = channel();
        mock_net.out.insert(i, sender);
        let handler = std::thread::spawn(
            move || {
                let mut node = Node{
                    network: MockNetworkComponent{
                        send_ch: mn_s,
                        recv_ch: recver,
                    },
                    index: i,
                    sk: partial_sk,
                    pk: pks.clone(),
                    total: n,  
                    role: if leader == i{
                            NodeRole::Leader
                        }else{
                            NodeRole::Follower
                        },
                };
                node.run();
            }
        );
        ths.push(
            handler
        );
    }

    let hm = std::thread::spawn(
        move || {
            while let Ok((target, msg)) = mock_net.mailbox.recv(){
                println!("mocknetwork: msg {} -> {}", msg.from, target);
                let kv = mock_net.out.get_mut(&target).unwrap();
                kv.send(msg).unwrap();
            }
        }
    );

    for h in ths{
        h.join().unwrap();
    }

    std::thread::sleep(std::time::Duration::from_millis(15000));
}

mod mvp{
    #[test]
    fn test_sig(){
    use threshold_crypto::SecretKey;
            
        let sk0 = SecretKey::random();
        let pk0 = sk0.public_key();
        let msg0 = b"Real news";
        assert!(pk0.verify(&sk0.sign(msg0), msg0));
        
        
        let sk1 = SecretKey::random();
        let msg1 = b"Fake news";
        assert!(!pk0.verify(&sk1.sign(msg0), msg0)); // Wrong key.
        assert!(!pk0.verify(&sk0.sign(msg1), msg0)); // Wrong message.
    }

    #[test]
    fn test_threshold_sig() {
        use threshold_crypto::SecretKeySet;
        use rand::thread_rng;

        let msg = b"hello, world!";
        let t = 2;   // for 4 nodes, need at least 3 sig

        let sks = SecretKeySet::random(t, &mut thread_rng());
        
        let sg1 = sks.secret_key_share(1).sign(msg);
        let sg2 = sks.secret_key_share(2).sign(msg);
        let sg3 = sks.secret_key_share(3).sign(msg);


        let sg_set = vec![(1, &sg1), (2, &sg2), (3, &sg3)];


        let pks = sks.public_keys();
        let msig = pks.combine_signatures(
            sg_set
        ).unwrap();

        assert!(pks.public_key().verify(&msig, msg));
        
    }

    #[test]
    #[should_panic]
    fn test_not_enough() {
        use threshold_crypto::SecretKeySet;
        use rand::thread_rng;

        let msg = b"hello, world!";
        let t = 2;   // for 4 nodes, need at least 3 sig

        let sks = SecretKeySet::random(t, &mut thread_rng());
        
        let sg1 = sks.secret_key_share(1).sign(msg);
        let sg2 = sks.secret_key_share(2).sign(msg);
        let sg3 = sks.secret_key_share(3).sign(msg);


        let sg_set = vec![(1, &sg1), (2, &sg2)];
        let pks = sks.public_keys();
        // 3 nodes at least.
        // should panic
        let msig = pks.combine_signatures(
            sg_set
        ).unwrap();
    }

}