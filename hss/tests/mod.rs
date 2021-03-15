//! test for HotstuffStorage

#[cfg(test)]
mod hss_test {

    use cryptokit::DefaultSignaturer;
    use hotstuff_rs::safety::machine::SafetyStorage;
    use hs_data::{form_chain, threshold_sign_kit, INIT_NODE, INIT_NODE_HASH};
    use hss::HotstuffStorage;
    use std::future::Future;

    fn init_logger() {
        use simplelog::*;
        let _ = CombinedLogger::init(vec![TermLogger::new(
            LevelFilter::Debug,
            ConfigBuilder::new()
                .add_filter_allow(format!("sqlx"))
                .build(),
            TerminalMode::Mixed,
        )]);
    }

    fn hss_with_mysql_enabled(
        token: String,
        mysql_addr: &'static str,
    ) -> impl Future<Output = HotstuffStorage> {
        let total = 4;
        let (sks, pks, _) = threshold_sign_kit(total, 2);
        let id = 1;
        let self_id = format!("replica-{}", id);
        let peers_addr = vec![
            (format!("replica-0"), format!("localhost:8000")),
            (format!("replica-1"), format!("localhost:8001")),
            (format!("replica-2"), format!("localhost:8002")),
            (format!("replica-3"), format!("localhost:8003")),
        ]
        .into_iter()
        .collect();

        let signaturer = DefaultSignaturer::new(id, pks.clone(), sks.secret_key_share(id));
        hss::init_hotstuff_storage(
            token,
            total,
            &INIT_NODE,
            &INIT_NODE_HASH,
            self_id,
            peers_addr,
            mysql_addr,
            signaturer,
        )
    }

    fn recover_hss_with_mysql_enabled(
        token: String,
        mysql_addr: &'static str,
    ) -> impl Future<Output = HotstuffStorage> {
        hss::HotstuffStorage::recover(token, mysql_addr)
    }

    async fn test_init_and_recover() {
        //
        //  Test:
        //      Flush all dirty data into MySQL.
        //
        //
        let mut hss = hss_with_mysql_enabled(
            format!("test"),
            "mysql://root:helloworld@localhost:3306/hotstuff_test_mocker",
        )
        .await;

        // drop and then create tables;
        hss.init().await;

        let (_, pk_set, vec_sks) = threshold_sign_kit(4, 2);
        let chain = form_chain(
            vec![
                "One summer afternoon",
                "Alice and her sister were enjoying the cool under a big tree",
                "Suddenly, a rabbit in a dress and a pocket watch ran past Alice.",
                "As the rabbit ran, he looked at his pocket watch and said, \"Late, late!\"",
                "Curious, Alice got up to chase the strange rabbit",
                "Alice followed the rabbit into a hole in the tree. ",
                "On a table in the tree hole, Alice saw a bottle. There is a label \"Drink Me\" on the bottle", 
            ],
            &vec_sks,
            &pk_set,
        );

        for (_, node) in &chain[..4] {
            println!("{}", node.height());
            hss.append_new_node(node);
        }

        hss.async_flush().await.unwrap();

        drop(hss);

        let mut hss2 = recover_hss_with_mysql_enabled(
            format!("test"),
            "mysql://root:helloworld@localhost:3306/hotstuff_test_mocker",
        )
        .await;

        for (_, node) in &chain[4..] {
            println!("{}", node.height());
            hss2.append_new_node(node);
        }
    }

    #[test]
    fn test_hss_init_and_recover() {
        init_logger();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(test_init_and_recover());
    }
}
