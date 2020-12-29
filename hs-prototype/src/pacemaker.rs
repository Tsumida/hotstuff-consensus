use std::collections::HashMap;

use tokio::sync::mpsc::{channel, Receiver, Sender};

use crate::safety::basic::*;
use crate::safety::msg::*;

pub trait Pacemaker {}

pub struct DefaultPacemaker {
    view: u64,
    tick: u64,
    deadline: u64,
    tick_ch: Receiver<(u64, u64)>,
    to_sm: Sender<InternalMsg>,
    to_net: Sender<InternalMsg>,
    peer_conf: HashMap<ReplicaID, String>,
    leader: Option<ReplicaID>,
}

impl DefaultPacemaker {
    pub fn new(
        peer_conf: HashMap<ReplicaID, String>,
        tick_ch: Receiver<(u64, u64)>,
        to_sm: Sender<InternalMsg>,
        to_net: Sender<InternalMsg>,
        leader: Option<ReplicaID>,
    ) -> Self {
        DefaultPacemaker {
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

    pub async fn run(&mut self) {
        let mut down = false;
        loop {
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
            if down {
                break;
            }
        }
    }

    fn touch_deadline(&self) -> bool {
        self.tick >= self.deadline
    }

    // TODO
    async fn view_change(&mut self) {
        let (sender, mut recvr) = channel(1);
        self.to_sm
            .send(InternalMsg::ViewChange(
                self.leader.as_ref().unwrap().clone(),
                self.view,
                sender,
            ))
            .await
            .unwrap();
    }
}
