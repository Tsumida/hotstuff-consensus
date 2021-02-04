use libp2p::{
    kad::{store::MemoryStore, KademliaEvent},
    swarm::NetworkBehaviourEventProcess,
    NetworkBehaviour,
};

use crate::pacemaker::PeerEventSender;
#[derive(NetworkBehaviour)]
// TODO: impl NetworkBehaviour.
// consider use kad to discover new node.
pub struct NetworkAdaptor {
    #[behaviour(ignore)]
    event_sender: PeerEventSender,
    kad: libp2p::kad::Kademlia<MemoryStore>,
}

impl NetworkBehaviourEventProcess<KademliaEvent> for NetworkAdaptor {
    fn inject_event(&mut self, event: KademliaEvent) {
        /*
        match event {
            KademliaEvent::QueryResult { id, result, stats } => {}
            KademliaEvent::RoutingUpdated {
                peer,
                addresses,
                old_peer,
            } => {}
            KademliaEvent::UnroutablePeer { peer } => {}
            KademliaEvent::RoutablePeer { peer, address } => {}
            KademliaEvent::PendingRoutablePeer { peer, address } => {}
        }*/
    }
}
