//! Pacemaker test.

/*
fn default_pacemaker_test_kit(storage: HotstuffStorage) {
    let n = 4;
    let id = format!("replica-{}", 1);
    let peers: HashMap<ReplicaID, String> = vec![
        (format!("replica-0"), format!("localhost:8000")),
        (format!("replica-1"), format!("localhost:8001")),
        (format!("replica-2"), format!("localhost:8002")),
        (format!("replica-3"), format!("localhost:8003")),
    ]
    .into_iter()
    .collect();

    let mut elector = RoundRobinLeaderElector::default();
    elector.init(peers.keys().cloned());

    let (machine_wrapper, machine_adaptor) = create_async_machine_pair();

    let pacemaker = Pacemaker {
        view: 0,
        id,
        elector,
        storage: (),
        sync_state: (),
        pending_prop: BinaryHeap::with_capacity(8),
        machine_adaptor: (),
        net_adaptor: (),
        timer: (),
        timeout_ch: (),
    };
}
*/
#[cfg(test)]
mod test {
    /*
    use std::collections::{BinaryHeap, HashMap};

    use hotstuff_rs::safety::machine::SafetyStorage;
    use hs_data::ReplicaID;
    use hss::HotstuffStorage;
    use pacemaker::{
        elector::RoundRobinLeaderElector,
        pacemaker::{AsyncMachineAdaptor, AsyncMachineWrapper, Pacemaker},
    };

    fn create_async_machine_pair<S: SafetyStorage>() -> (AsyncMachineWrapper<S>, AsyncMachineAdaptor)
    {
        unimplemented!()
    }


    #[test]
    fn test_bootstrap() {}

    #[test]
    fn test_leader_election() {}

    #[test]
    fn test_view_change() {}

    #[test]
    fn test_local_timeout() {}*/
}
