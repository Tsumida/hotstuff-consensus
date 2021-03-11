//!

use std::{collections::HashMap, io, net::SocketAddr};

use crate::{
    data::PeerEvent,
    pacemaker::{CtlSender, PeerEventRecvr, PeerEventSender},
};
use hs_data::ReplicaID;
use log::{error, info};
use tokio::{io::AsyncReadExt, net::TcpSocket};

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

impl NetworkAdaptor {
    // todo
    async fn connect_tcp_proxy(&self) -> io::Result<()> {
        let proxy_addr = if let NetworkMode::TcpProxy(addr) = self.mode {
            addr
        } else {
            panic!();
        };

        // tcp connection
        let sk = TcpSocket::new_v4()?;
        sk.bind(self.self_addr)?;
        let mut stream = sk.connect(proxy_addr).await?;
        info!("forwarding task starts");

        // init state.
        let event_sender = self.event_sender.clone();
        let max_limit = self.max_transport_limit;
        let mut buf: Vec<u8> = Vec::with_capacity(max_limit);
        let mut stop_recvr = self.ctrl_ch.subscribe();
        let mut quit = false;

        tokio::spawn(async move {
            while !quit {
                tokio::select! {
                    _ = stop_recvr.recv() => {
                        quit = false;
                    },
                    // process incoming peer event
                    Ok(byte_len) = stream.read_u32() => {
                        buf.resize_with(byte_len as usize, Default::default);
                        if let Err(e) = stream.read_exact(&mut buf).await{
                            error!{"{:?}", e};
                            continue;
                        }
                        match serde_json::from_slice::<PeerEvent>(&buf){
                            Ok(event) => {
                                event_sender.send(event).await.unwrap();
                            },
                            Err(e) => {
                                error!("{:?}", e);
                            }
                        }
                    },
                }
            }
            info!("forwarding task done");
        });
        Ok(())
    }
}
