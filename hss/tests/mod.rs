//! test for HotstuffStorage

#[cfg(test)]
mod hss_test {

    use cryptokit::DefaultSignaturer;
    use hotstuff_rs::safety::machine::SafetyStorage;
    use hs_data::{
        combined_sign_from_vec_u8, threshold_sign_kit, CombinedSign, GenericQC, NodeHash, Sign,
        TreeNode, Txn, INIT_NODE, INIT_NODE_HASH, PK, SK,
    };
    use hss::HotstuffStorage;
    use std::future::Future;

    fn init_logger() {
        use simplelog::*;
        let _ = CombinedLogger::init(vec![TermLogger::new(
            LevelFilter::Debug,
            Config::default(),
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
        hss::init_hotstuff_storage(token, total, self_id, peers_addr, mysql_addr, signaturer)
    }

    fn form_combined_sign(node: &TreeNode, sks: &Vec<(usize, SK)>, pks: &PK) -> CombinedSign {
        let node_bytes = node.to_be_bytes();
        let signs = sks
            .iter()
            .map(|(i, sk)| (*i, sk.sign(&node_bytes)))
            .collect::<Vec<(usize, Sign)>>();
        pks.combine_signatures(signs.iter().map(|(i, s)| (*i, s)))
            .unwrap()
    }

    fn form_chain(
        txn: Vec<&'static str>,
        vec_sks: &Vec<(usize, SK)>,
        pk_set: &PK,
    ) -> Vec<(NodeHash, TreeNode)> {
        let mut v = vec![];
        let mut height = 0;
        let mut parent_hash = INIT_NODE_HASH.clone();
        let mut parent_node = INIT_NODE.clone();
        for tx in txn {
            let combined_sign = form_combined_sign(&parent_node, vec_sks, pk_set);
            let justify = GenericQC::new(height, &parent_hash, &combined_sign);
            let (node, hash) =
                TreeNode::node_and_hash(vec![&Txn::new(tx)], height + 1, &parent_hash, &justify);
            v.push((hash.as_ref().clone(), node.as_ref().clone()));

            height += 1;
            parent_hash = *hash;
            parent_node = *node;
        }
        v
    }

    async fn test_flush() {
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
        hss.init().await;

        let (_, pk_set, vec_sks) = threshold_sign_kit(4, 2);
        let chain = form_chain(
            vec![
                "One summer afternoon",
                "Alice and her sister were enjoying the cool under a big tree",
                "Suddenly, a rabbit in a dress and a pocket watch ran past Alice.",
                "As the rabbit ran, he looked at his pocket watch and said, \"Late, late!\"",
                "Curious, Alice got up to chase the strange rabbit",
            ],
            &vec_sks,
            &pk_set,
        );

        for (_, node) in &chain {
            println!("{}", node.height());
            hss.append_new_node(node);
        }

        hss.async_flush().await.unwrap();
    }

    #[test]
    fn test_hss_flush() {
        init_logger();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(test_flush());
    }
}
