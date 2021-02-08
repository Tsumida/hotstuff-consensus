use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::mpsc::sync_channel,
    time::Duration,
};

use futures::future::join;
use hotstuff_rs::msg::Context;
use libp2p::{
    gossipsub::{GossipsubConfig, GossipsubConfigBuilder, GossipsubMessage, IdentTopic, MessageId},
    identity::Keypair,
    Multiaddr, PeerId,
};
use pacemaker::{
    data::PeerEvent,
    network::{network_component_pair, NetworkAdaptor},
};
use simplelog::*;
use tokio::{runtime::Runtime, spawn};

fn init_logger() {
    let _ = CombinedLogger::init(vec![TermLogger::new(
        LevelFilter::Debug,
        Config::default(),
        TerminalMode::Mixed,
    )]);
}

#[test]
fn test_gossip_bootstrap() {
    init_logger();
    let topic = IdentTopic::new("test-gossip");

    let keys: Vec<(Keypair, Multiaddr)> = vec![
        (
            Keypair::generate_secp256k1(),
            "/ip4/127.0.0.1/tcp/6666".parse().unwrap(),
        ),
        (
            Keypair::generate_secp256k1(),
            "/ip4/127.0.0.1/tcp/8888".parse().unwrap(),
        ),
    ];

    let peer_ids = keys
        .iter()
        .map(|(k, addr)| (PeerId::from_public_key(k.public()), addr.clone()))
        .collect::<Vec<_>>();

    let (na1, nw1) = network_component_pair(
        keys.get(0).unwrap().0.clone(),
        &peer_ids,
        default_conf(),
        topic.clone(),
        2,
    );
    let (na2, nw2) = network_component_pair(
        keys.get(1).unwrap().0.clone(),
        &peer_ids,
        default_conf(),
        topic.clone(),
        2,
    );

    let ctx1 = Context {
        from: format!("na1"),
        view: 0,
    };

    let ctx2 = Context {
        from: format!("na2"),
        view: 0,
    };

    let (sender, recvr) = sync_channel(2);

    Runtime::new().unwrap().block_on(async move {
        spawn(nw1.run("/ip4/127.0.0.1/tcp/6666".parse().unwrap()));
        tokio::time::sleep(Duration::from_secs(2)).await;

        spawn(nw2.run("/ip4/127.0.0.1/tcp/8888".parse().unwrap()));
        tokio::time::sleep(Duration::from_secs(2)).await;

        let sender = sender;
        join(
            assert_bootstrap_ping(
                na1,
                PeerEvent::Ping {
                    ctx: ctx1,
                    cont: format!("hello, im na1"),
                },
                sender.clone(),
            ),
            assert_bootstrap_ping(
                na2,
                PeerEvent::Ping {
                    ctx: ctx2,
                    cont: format!("hello, im na2"),
                },
                sender.clone(),
            ),
        )
        .await;
    });

    for e in recvr.into_iter().take(2) {
        if let PeerEvent::Ping { ctx, cont } = e {
            assert!(
                (&ctx.from == "na1" && &cont == "hello, im na1")
                    || (&ctx.from == "na2" && &cont == "hello, im na2")
            );
        } else {
            assert!(false)
        }
    }
}

async fn assert_bootstrap_ping(
    mut na: NetworkAdaptor,
    pe: PeerEvent,
    sender: std::sync::mpsc::SyncSender<PeerEvent>,
) {
    na.event_sender.send(pe).await.expect("failed");
    sender.send(na.event_recvr.recv().await.unwrap()).unwrap();
    na.stop_sender.send(()).unwrap();
}

fn default_conf() -> GossipsubConfig {
    let dedup_fn = |msg: &GossipsubMessage| {
        let mut hasher = DefaultHasher::new();
        msg.data.hash(&mut hasher);
        MessageId::from(hasher.finish().to_string())
    };

    GossipsubConfigBuilder::default()
        .heartbeat_interval(Duration::from_secs(5))
        .message_id_fn(dedup_fn)
        .build()
        .expect("failed to build")
}
