//! Pacemaker test.
#[cfg(test)]
mod test {
    use cryptokit::DefaultSignaturer;
    use hotstuff_rs::safety::{machine::Machine, voter::Voter};
    use hs_data::{
        form_chain, msg::Context, threshold_sign_kit, ReplicaID, INIT_NODE, INIT_NODE_HASH, SK,
    };
    use hss::HotstuffStorage;
    use std::{collections::HashMap, net::SocketAddr, str::FromStr, sync::Arc, time::Duration};
    use threshold_crypto::{PublicKeySet, SecretKeySet};

    use pacemaker::{
        data::PeerEvent,
        elector::RoundRobinLeaderElector,
        network::{NetworkAdaptor, NetworkMode},
        pacemaker::{Pacemaker, TchanR, TchanS},
    };

    use crate::utils::init_pm_logger;

    static TX: &'static [&'static str] = &[
        "One summer afternoon",
        "Alice and her sister were enjoying the cool under a big tree",
        "Suddenly, a rabbit in a dress and a pocket watch ran past Alice.",
        "As the rabbit ran, he looked at his pocket watch and said, \"Late, late!\"",
        "Curious, Alice got up to chase the strange rabbit",
        "Alice followed the rabbit into a hole in the tree. ",
        "On a table in the tree hole, Alice saw a bottle. There is a label \"Drink Me\" on the bottle", 
    ];

    const CHANNEL_SIZE: usize = 128;

    struct MockerNetwork {
        token: String,
        pe_sender: TchanS<PeerEvent>,
        pe_recvr: TchanR<PeerEvent>,
        sks: SecretKeySet,
        pks: PublicKeySet,
        vec_sks: Vec<(usize, SK)>,
    }

    async fn new_mocker_with_mysql_enabled(
        n: usize,
        token: String,
        mysql_addr: &str,
    ) -> (MockerNetwork, Pacemaker<HotstuffStorage>) {
        assert!(n >= 1);
        let threshold = (n << 1) / 3;
        let sign_id = 0;
        let id = format!("replica-{}", sign_id);
        let peers_addr: HashMap<ReplicaID, String> = (0..n)
            .map(|i| (format!("replica-{}", i), format!("127.0.0.1:{}", 8800 + i)))
            .collect();

        let (sks, pks, vec_sks) = threshold_sign_kit(n, threshold);
        let signaturer = Arc::new(DefaultSignaturer::new(
            sign_id,
            pks.clone(),
            vec_sks.get(sign_id).unwrap().1.clone(),
        ));

        let mut elector = RoundRobinLeaderElector::default();
        elector.init(peers_addr.keys().cloned());

        let voter = Voter::new(threshold, signaturer.as_ref().clone());

        let (pe_sender, event_recvr) = tokio::sync::mpsc::channel(CHANNEL_SIZE);
        let (event_sender, pe_recvr) = tokio::sync::mpsc::channel(CHANNEL_SIZE);

        // For cancelling network task.
        let (ctrl_ch, _) = tokio::sync::broadcast::channel(4);

        let net_adaptor = NetworkAdaptor {
            event_sender,
            event_recvr,
            ctrl_ch,
            self_addr: SocketAddr::from_str(peers_addr.get(&id).unwrap()).unwrap(),
            max_transport_limit: 2 << 20, // 2 MB
            mode: NetworkMode::Test,
        };

        let storage: HotstuffStorage = hss::init_hotstuff_storage(
            token.clone(),
            n,
            &INIT_NODE,
            &INIT_NODE_HASH,
            id.clone(),
            peers_addr,
            mysql_addr,
            signaturer.as_ref().clone(),
        )
        .await;

        let machine = Machine::new(voter, id.clone(), n, None, storage);

        (
            MockerNetwork {
                token,
                pe_sender,
                pe_recvr,
                sks,
                pks,
                vec_sks,
            },
            Pacemaker::new(id, elector, machine, net_adaptor),
        )
    }

    #[test]
    fn test_bootstrap() {
        //
        // Test: PM, M & HSS cooperating
        //
        //
        //      MPM ------> PM ------> M ------> HSS ------> MySQL
        //                 | |         |
        //         <-------   <--------
        //        PE(Accept)  Ready(Sign)
        //
        // Disable branch sync

        init_pm_logger("./test-output/test-bootstrap.log");

        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async move {
                let (mut mpm, mut pm) = new_mocker_with_mysql_enabled(
                    4,
                    format!("test"),
                    crate::utils::TEST_MYSQL_ADDR,
                )
                .await;

                let (quit_sender, quit_ch) = tokio::sync::mpsc::channel(1);

                let handler = tokio::spawn(async move {
                    pm.run(quit_ch).await.unwrap();
                });

                let chain = form_chain(TX.iter().take(4).map(|s| *s), &mpm.vec_sks, &mpm.pks);
                let from = format!("replica-1");
                for (_, prop) in chain {
                    mpm.pe_sender
                        .send(PeerEvent::NewProposal {
                            ctx: Context {
                                view: prop.height(),
                                from: from.clone(),
                            },
                            prop: Box::new(prop),
                        })
                        .await
                        .unwrap();
                }

                quit_sender.send(()).await.unwrap();

                // verify
                let mut flag = true;
                while let Some(ready) = mpm.pe_recvr.recv().await {
                    if let PeerEvent::AcceptProposal { .. } = ready {
                    } else {
                        flag = false;
                        break;
                    }
                }

                handler.await.unwrap();
                assert!(flag);
            });
    }

    #[test]
    fn test_local_timeout() {}

    #[test]
    fn test_branch_sync() {}
}
