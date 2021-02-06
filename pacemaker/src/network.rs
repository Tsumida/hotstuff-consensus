use crate::{
    data::PeerEvent,
    pacemaker::{CtlSender, PeerEventRecvr, PeerEventSender, TchanS},
};
use gossipsub::{GossipsubConfig, GossipsubConfigBuilder, IdentTopic, MessageAuthenticity};
use libp2p::{
    gossipsub::{self, GossipsubMessage, MessageId},
    identity::{self},
    PeerId,
};
use libp2p::{
    gossipsub::{Gossipsub, GossipsubEvent},
    swarm::NetworkBehaviourEventProcess,
    Multiaddr, NetworkBehaviour,
};
use log::error;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};
pub struct NetworkAdaptor {
    pub event_sender: PeerEventSender,
    pub event_recvr: PeerEventRecvr,
}

#[derive(NetworkBehaviour)]
pub struct NetworkWrapper {
    // Event from other peers.
    #[behaviour(ignore)]
    net_recvr: PeerEventRecvr,
    // Event to other peers.
    #[behaviour(ignore)]
    net_sender: PeerEventSender,

    #[behaviour(ignore)]
    stop_sender: CtlSender,

    gossip: Gossipsub,
}

impl NetworkBehaviourEventProcess<GossipsubEvent> for NetworkWrapper {
    fn inject_event(&mut self, event: GossipsubEvent) {
        match event {
            GossipsubEvent::Message {
                propagation_source,
                message_id,
                message,
            } => {}
            GossipsubEvent::Subscribed { peer_id, topic } => {}
            GossipsubEvent::Unsubscribed { peer_id, topic } => {}
        }
    }
}

impl NetworkWrapper {
    pub async fn run(
        mut self,
        addr: Multiaddr,
        gossip_topic: IdentTopic,
        gossip_conf: GossipsubConfig,
    ) -> () {
        let local_key_1 = identity::Keypair::generate_ed25519();
        let peer_id_1 = PeerId::from_public_key(local_key_1.public());

        let transport = libp2p::build_development_transport(local_key_1.clone()).unwrap();

        let mut dedup_fn = |msg: &GossipsubMessage| {
            let mut hasher = DefaultHasher::new();
            msg.data.hash(&mut hasher);
            MessageId::from(hasher.finish().to_string())
        };

        let mut gossip: gossipsub::Gossipsub =
            gossipsub::Gossipsub::new(MessageAuthenticity::Signed(local_key_1), gossip_conf)
                .expect("failed to create gossipsub");
        gossip.subscribe(&gossip_topic).unwrap();

        let mut swarm = libp2p::swarm::Swarm::new(transport, gossip, peer_id_1);

        // listening
        if let Err(e) = libp2p::Swarm::listen_on(&mut swarm, addr) {
            error!("{}", e.to_string());
        }

        let mut stop_recvr = self.stop_sender.subscribe();
        let topic = gossip_topic.clone();
        tokio::spawn(async move {
            let mut quit = false;
            let topic = topic;
            while !quit {
                tokio::select! {
                    Ok(()) = stop_recvr.recv() => {
                        quit = true;
                    }
                    gossip_event = swarm.next() => {
                        match gossip_event{
                            GossipsubEvent::Message { propagation_source, message_id, message } => {}
                            _ => {}
                        }
                    }
                    Some(peer_event) = self.net_recvr.recv() => {
                        match swarm.publish(topic.clone(), peer_event.to_be_bytes()) {
                            Ok(_) => {}
                            Err(_) => {}
                        }
                    }
                }
            }
        });
    }
}

pub fn gossip_pair(gossip: Gossipsub, ch_size: usize) -> (NetworkAdaptor, NetworkWrapper) {
    let (event_sender, event_recvr) = tokio::sync::mpsc::channel(ch_size);
    let (net_sender, net_recvr) = tokio::sync::mpsc::channel(ch_size);
    let (stop_sender, _) = tokio::sync::broadcast::channel(1);

    let na = NetworkAdaptor {
        event_sender,
        event_recvr,
    };

    let nw = NetworkWrapper {
        net_recvr,
        net_sender,
        gossip,
        stop_sender,
    };

    (na, nw)
}
