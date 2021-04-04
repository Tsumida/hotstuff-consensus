//!

use std::{collections::HashMap, net::SocketAddr};

use crate::pacemaker::{CtlSender, PeerEventRecvr, PeerEventSender};
use hs_data::ReplicaID;

pub enum NetworkMode {
    Test,
    // NetworkAdaptor connects to other peers directly using TCP.
    Direct(HashMap<ReplicaID, SocketAddr>),
    // NetworkAdaptor forwards events to a tcp proxy who knows how to diliver them.
    TcpProxy(SocketAddr),

    // http proxy
    HttpProxy,
}

pub struct NetworkAdaptor {
    // events to other peers.
    pub event_sender: PeerEventSender,
    // from other peers.
    pub event_recvr: PeerEventRecvr,
    pub mode: NetworkMode,
    pub ctrl_ch: CtlSender,
    pub self_addr: SocketAddr,
    // Maxium Byte for each PeerEvent.
    pub max_transport_limit: usize,
}
