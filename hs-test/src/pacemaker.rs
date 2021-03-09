//! Pacemaker test.
#[cfg(test)]
mod test {
    use cryptokit::DefaultSignaturer;
    use hotstuff_rs::safety::{machine::Machine, voter::Voter};
    use hs_data::{
        form_chain, msg::Context, threshold_sign_kit, ReplicaID, SignKit, TreeNode, INIT_NODE,
        INIT_NODE_HASH, INIT_QC, SK,
    };
    use hss::HotstuffStorage;
    use log::error;
    use std::{collections::HashMap, net::SocketAddr, str::FromStr, sync::Arc, time::Duration};
    use threshold_crypto::{PublicKeySet, SecretKeySet};

    use pacemaker::{
        data::{PeerEvent, TimeoutCertificate},
        elector::RoundRobinLeaderElector,
        network::{NetworkAdaptor, NetworkMode},
        pacemaker::{Pacemaker, TchanR, TchanS},
    };

    use crate::utils::{init_logger, TEST_MYSQL_ADDR};

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
    const DEFAULT_LEADER: usize = 1;
    const DEFAULT_TESTEE: usize = 0;
    const DEFAULT_SIZE: usize = 4;
    const DEFAULT_TH: usize = (DEFAULT_SIZE << 1) / 3;

    struct MockerNetwork {
        token: String,
        pe_sender: TchanS<PeerEvent>,
        pe_recvr: TchanR<PeerEvent>,
        sks: SecretKeySet,
        pks: PublicKeySet,
        vec_sks: Vec<(usize, SK)>,
    }

    impl MockerNetwork {
        async fn propose_all<'a>(
            &mut self,
            from: String,
            props: impl IntoIterator<Item = &'a TreeNode>,
        ) {
            for prop in props.into_iter() {
                self.pe_sender
                    .send(PeerEvent::NewProposal {
                        ctx: Context::single(
                            format!("replica-{}", DEFAULT_LEADER),
                            format!("replica-{}", DEFAULT_TESTEE),
                            prop.height(),
                        ),
                        prop: Box::new(prop.clone()),
                    })
                    .await
                    .unwrap();
            }
        }

        /// Verify each PeerEvent and return false once meet a event with `v(&event) == false`.
        async fn vertify_output_sequence(&mut self, mut v: impl FnMut(&PeerEvent) -> bool) -> bool {
            let mut flag = true;
            while let Some(ready) = self.pe_recvr.recv().await {
                flag &= v(&ready);
                if !flag {
                    break;
                }
            }

            flag
        }
    }

    async fn new_mocker_with_mysql_enabled(
        n: usize,
        token: String,
        mysql_addr: &str,
    ) -> (MockerNetwork, Pacemaker<HotstuffStorage>) {
        assert!(n >= 1);
        let threshold = (n << 1) / 3;
        let sign_id = DEFAULT_TESTEE;
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
            Pacemaker::new(
                id,
                elector,
                machine,
                net_adaptor,
                signaturer.as_ref().clone(),
            ),
        )
    }

    async fn spawn_pm(
        mysql_addr: &str,
    ) -> (
        MockerNetwork,
        tokio::sync::mpsc::Sender<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let (mpm, mut pm) = new_mocker_with_mysql_enabled(4, format!("test"), mysql_addr).await;

        let (quit_sender, quit_ch) = tokio::sync::mpsc::channel(1);

        let handler = tokio::spawn(async move {
            pm.run(quit_ch).await.unwrap();
        });

        (mpm, quit_sender, handler)
    }

    #[test]
    fn test_responsiveness() {
        //
        // Test:
        //      PM, M & HSS cooperating. After accpeting new proposal, the testee will quickly go to next view.
        //
        //      MPM ------> PM ------> M ------> HSS ------> MySQL
        //                 | |         |
        //         <-------   <--------
        //        PE(Accept)  Ready(Sign)
        //        PE(Timeout)
        //

        init_logger("./test-output/test-responsiveness.log");

        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async move {
                let (mut mpm, quit_sender, handler) = spawn_pm(TEST_MYSQL_ADDR).await;

                let chain = form_chain(TX.iter().take(4).map(|s| *s), &mpm.vec_sks, &mpm.pks);
                let from = format!("replica-1");

                mpm.propose_all(from, chain.iter().map(|(_, p)| p)).await;

                // let pm process events.
                tokio::time::sleep(Duration::from_secs(8)).await;
                quit_sender.send(()).await.unwrap();

                let mut view = 1;
                let flag = mpm
                    .vertify_output_sequence(|event| match event {
                        // first accept and then timeout
                        PeerEvent::AcceptProposal { prop, sign, .. } => {
                            let res = prop.height() == view && sign.is_some();
                            view += 1;
                            res
                        }
                        PeerEvent::Timeout { tc, .. } => tc.view() < view,
                        PeerEvent::NewView { .. } => true,
                        p => {
                            error!("unexcepted event: {:?}", p);
                            false
                        }
                    })
                    .await;

                handler.await.unwrap();
                assert!(flag);
            });
    }

    #[test]
    #[ignore = "too long"]
    fn test_local_timeout() {
        //
        // Test:
        //      Wait for local timeout.
        //
        init_logger("./test-output/test-local-timeout.log");

        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async move {
                let (mut mpm, quit_sender, handler) = spawn_pm(TEST_MYSQL_ADDR).await;

                tokio::time::sleep(Duration::from_secs(30 * 2)).await;
                quit_sender.send(()).await.unwrap();

                let mut cnt = 0;
                let flag = mpm
                    .vertify_output_sequence(|event| match event {
                        PeerEvent::Timeout { tc, .. } => {
                            if cnt < tc.view() {
                                cnt = tc.view();
                                true
                            } else {
                                false
                            }
                        }
                        PeerEvent::NewView { .. } => true,
                        _ => false,
                    })
                    .await;

                handler.await.unwrap();
                assert!(flag);
            });
    }

    #[test]
    fn test_remote_timeout() {
        //
        // Test:
        //      Testee receives n-f PartialTC(view=v), and then timeout and go to view=v+1
        //
        init_logger("./test-output/test-remote-timeout.log");

        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async move {
                let (mut mpm, quit_sender, handler) = spawn_pm(TEST_MYSQL_ADDR).await;

                // create n-f PartialTC with view=v.
                let v = 10u64;
                for pe in mpm
                    .vec_sks
                    .iter()
                    .filter(|(id, _)| id != &DEFAULT_TESTEE)
                    .map(|(id, sks)| {
                        let from = format!("replica-{}", id);
                        let tc = TimeoutCertificate::new(
                            from.clone(),
                            v,
                            SignKit::new(*id, sks.sign(&v.to_be_bytes())),
                            INIT_QC.clone(),
                        );
                        PeerEvent::Timeout {
                            ctx: Context::broadcast(from, v),
                            tc,
                        }
                    })
                {
                    mpm.pe_sender.send(pe).await.unwrap();
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }

                quit_sender.send(()).await.unwrap();

                let flag = mpm
                    .vertify_output_sequence(|event| match event {
                        PeerEvent::Timeout { tc, .. } => tc.view() == v + 1,
                        PeerEvent::NewView { .. } => true,
                        _ => false,
                    })
                    .await;

                handler.await.unwrap();
                assert!(flag);
            });
    }

    #[test]
    #[ignore = "unimpl"]
    fn test_branch_sync() {}
}
