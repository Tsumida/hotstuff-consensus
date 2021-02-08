use crate::pacemaker::{CtlRecvr, CtlSender, PeerEventRecvr, PeerEventSender};
use gossipsub::{GossipsubConfig, IdentTopic, MessageAuthenticity};
use identity::Keypair;
use libp2p::{
    build_development_transport, gossipsub, identity,
    kad::{store::MemoryStore, Kademlia, KademliaEvent},
    swarm::{self, NetworkBehaviour},
    PeerId,
};
use libp2p::{
    gossipsub::{Gossipsub, GossipsubEvent},
    swarm::NetworkBehaviourEventProcess,
    Multiaddr, NetworkBehaviour,
};
use log::{error, info};
use std::time::Duration;
pub struct NetworkAdaptor {
    pub event_sender: PeerEventSender,
    pub event_recvr: PeerEventRecvr,
    pub stop_sender: CtlSender,

    pub peer_id: PeerId,
}

#[derive(NetworkBehaviour)]
pub struct NetworkWrapper {
    // Events from local pacemaker were sent to other peer
    #[behaviour(ignore)]
    net_recvr: Option<PeerEventRecvr>,
    // Event from other peer
    #[behaviour(ignore)]
    net_sender: PeerEventSender,

    #[behaviour(ignore)]
    stop_recvr: Option<CtlRecvr>,

    #[behaviour(ignore)]
    local_key: Keypair,

    #[behaviour(ignore)]
    peer_id: PeerId,

    #[behaviour(ignore)]
    topic: IdentTopic,

    gossip: Gossipsub,
    kad: Kademlia<MemoryStore>,
}

impl NetworkBehaviourEventProcess<GossipsubEvent> for NetworkWrapper {
    fn inject_event(&mut self, event: GossipsubEvent) {
        match event {
            GossipsubEvent::Message { message, .. } => {
                let sender = self.net_sender.clone();
                if let Ok(pe) = serde_json::from_slice(&message.data) {
                    tokio::spawn(async move {
                        if let Err(e) = sender.send_timeout(pe, Duration::from_secs(1)).await {
                            error!("{}", e.to_string());
                        }
                    });
                } else {
                    error!("deserialization falied. ");
                }
            }
            _ => {}
        }
    }
}

impl NetworkBehaviourEventProcess<KademliaEvent> for NetworkWrapper {
    fn inject_event(&mut self, event: KademliaEvent) {
        info!("--------------{:?}", event);
    }
}

impl NetworkWrapper {
    pub async fn run(mut self, addr: Multiaddr) -> () {
        let topic = self.topic.clone();
        let mut stop_recvr = self.stop_recvr.take().unwrap();
        let mut net_recvr = self.net_recvr.take().unwrap();
        let local_key = self.local_key.clone();

        tokio::spawn(async move {
            info!("peer-id = {}", &self.peer_id);
            let transport = build_development_transport(local_key.clone()).unwrap();
            let mut swarm = libp2p::swarm::Swarm::new(
                transport,
                self,
                PeerId::from_public_key(local_key.public()),
            );

            // listening
            if let Err(e) = libp2p::Swarm::listen_on(&mut swarm, addr) {
                error!("{}", e.to_string());
            }

            let mut quit = false;
            let topic = topic;

            swarm.kad.bootstrap().unwrap();

            info!("network component up");
            while !quit {
                tokio::select! {
                    _ = stop_recvr.recv() => {
                        quit = true;
                    },
                    Some(peer_event) = net_recvr.recv() => {
                        if let Ok(buf) = serde_json::to_vec(&peer_event){
                            if let Err(e) = swarm.gossip.publish(topic.clone(), buf){
                                error!("failed to publish msg: {:?}", e);
                            }
                        }
                    },
                }
            }
            info!("network component down");
        });
    }
}

pub fn network_component_pair<'a>(
    local_key_1: Keypair,
    peer_ids: impl IntoIterator<Item = &'a (PeerId, Multiaddr)>,
    gossip_conf: GossipsubConfig,
    gossip_topic: IdentTopic,
    ch_size: usize,
) -> (NetworkAdaptor, NetworkWrapper) {
    let (event_sender, net_recvr) = tokio::sync::mpsc::channel(ch_size);
    let (net_sender, event_recvr) = tokio::sync::mpsc::channel(ch_size);
    let (stop_sender, stop_recvr) = tokio::sync::broadcast::channel(1);

    let peer_id_1 = PeerId::from_public_key(local_key_1.public());

    let store = MemoryStore::new(peer_id_1.clone());
    let mut kad = Kademlia::new(peer_id_1.clone(), store);

    for (peer, address) in peer_ids.into_iter() {
        if peer != &peer_id_1 {
            kad.add_address(peer, address.clone());
            info!("add peer {:?}", address);
        }
    }

    let mut gossip = gossipsub::Gossipsub::new(
        MessageAuthenticity::Signed(local_key_1.clone()),
        gossip_conf,
    )
    .expect("failed to create gossipsub");

    gossip.subscribe(&gossip_topic).unwrap();

    let na = NetworkAdaptor {
        event_sender,
        event_recvr,
        stop_sender,
        peer_id: peer_id_1.clone(),
    };

    let nw = NetworkWrapper {
        net_recvr: Some(net_recvr),
        net_sender,
        gossip,
        stop_recvr: Some(stop_recvr),
        local_key: local_key_1,
        topic: gossip_topic,
        peer_id: peer_id_1.clone(),
        kad,
    };

    (na, nw)
}
