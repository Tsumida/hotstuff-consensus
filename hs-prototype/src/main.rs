//!
//! Event-driven HotStuff prototype
//! 

use tokio::{
    prelude::*, 
    sync::{
        mpsc, 
    },
    runtime::Runtime, 
};

use log::{
    info,
};

use simplelog::*; 

mod state_machine; 
use state_machine::{
    StateMachine, 
    MsgToSM, 
    Ready, 
};
mod utils; 
use utils::threshold_sign_kit;
mod basic; 
use basic::*;


#[test]
#[ignore = "dev"]
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

pub struct HotStuffNode{
    node_id: String, 

    down: mpsc::Receiver<()>, 

    // stateMachine
    sm_out: mpsc::Receiver<Ready>,
    sm_in: mpsc::Sender<MsgToSM>, 
}

impl HotStuffNode{
    // 
    async fn run(&mut self){
        let mut down = false; 
        loop{
            tokio::select!{
                // state machine 
                Some(ready) = self.sm_out.recv() => {
                    self.process_ready(ready).await;
                }, 
                Some(()) = self.down.recv() => {
                    down = true;
                }
            }
            if down{
                break;
            }
        }
    }

    /// Process Ready Msg from StateMachine.
    async fn process_ready(&mut self, ready: Ready){
        match ready{
            Ready::UpdateQCHigh(qc) => {
                // Notify PM
            }, 
            Ready::VoteReply(hash, partial_sign) => {
                // Respond to rpc requests. 
            }, 
            Ready::State(_) => {
                // let (ss, hs) = state.as_ref(); 
                // stablization or .. 
            }, 
            _ => {}, 
        }
    }
}

#[tokio::main]
async fn main() {
    CombinedLogger::init(
        vec![
           TermLogger::new(LevelFilter::Debug, Config::default(), TerminalMode::Mixed),  
           // WriteLogger::new(LevelFilter::Warn, Config::default(), File::create("./warnlog.log").unwrap()),
        ]
    ).unwrap(); 
    info!("running demo (4 nodes)");
    let f = 1; 
    let n = 3 * f + 1;

    let (sks, pks, _) = utils::threshold_sign_kit(n, 2*f); 
    let (mut sm, ready_ch, msg_ch, _) = StateMachine::new(format!("node-1"), sks.secret_key_share(0), pks.clone());
    let (mut down_s, down_r) = mpsc::channel(1);
    let mut node = HotStuffNode{
        node_id: "node-0".to_string(), 
        sm_in: msg_ch, 
        sm_out: ready_ch, 
        down: down_r, 
    };

    tokio::spawn(
        async move{
            tokio::time::delay_for(std::time::Duration::from_millis(2 * 1000)).await; 
            down_s.send(()).await.unwrap();
            info!("demo done."); 
        }
    );

    tokio::spawn(
        async move{
            sm.run().await; 
        }
    );

    tokio::spawn(
        async move {
            node.run().await;
        }
    ).await.unwrap();

}