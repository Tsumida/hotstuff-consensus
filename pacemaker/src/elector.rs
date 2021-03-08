//! leader election.

use hs_data::{ReplicaID, ViewNumber};

pub struct RoundRobinLeaderElector {
    // next_leader = (this_leader + 1) % peers_nums
    round_mapper: Vec<ReplicaID>,
}

impl RoundRobinLeaderElector {
    pub fn init(&mut self, replicas: impl IntoIterator<Item = ReplicaID>) {
        let mut tmp = replicas.into_iter().collect::<Vec<ReplicaID>>();
        tmp.sort();
        tmp.dedup();
        self.round_mapper = tmp;
    }

    pub fn get_leader(&self, view: ViewNumber) -> &ReplicaID {
        // leader number = hash(view) % peer_num
        self.round_mapper
            .get(crate::utils::view_hash(view, self.round_mapper.len()))
            .unwrap()
    }
}

impl Default for RoundRobinLeaderElector {
    fn default() -> Self {
        Self {
            round_mapper: Vec::new(),
        }
    }
}
