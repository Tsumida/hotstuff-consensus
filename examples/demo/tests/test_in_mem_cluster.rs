//! Test for in-memory cluster (without MySQL).

#[cfg(test)]
mod test_bin {
    use std::process::{Child, Command};

    const BIN_TEST_KIT: &str = env!("CARGO_BIN_EXE_test_kit");
    const BIN_NODE_ADMIN: &str = env!("CARGO_BIN_EXE_node_admin");

    #[test]
    fn test_bin_4_correct_nodes() {
        //
        //  Test:
        //      4 correct nodes with roundrobin elector.
        //  Steps:
        //      1. Each node submit 10 proposals. (40 proposals total)
        //      2. All node stops when come to view 41.
        //

        // generate node config.
        assert!(Command::new(BIN_TEST_KIT)
            .args(&[
                "node-config",
                "--with-name",
                "-m",
                "-n=4",
                "-o=./test-output",
            ])
            .output()
            .expect("failed to gen config")
            .status
            .success());

        // create 4 nodes.

        let childs = ["Alice", "Bob", "Carol", "Dave"]
            .iter()
            .filter_map(|name| {
                Command::new(BIN_NODE_ADMIN)
                    .arg(format!(
                        "--config=./test-output/{}-config.yml",
                        name.to_lowercase()
                    ))
                    .spawn()
                    .ok()
            })
            .collect::<Vec<Child>>();

        if childs.len() != 4 {
            childs.into_iter().for_each(|mut c| {
                c.kill().expect("failed to kill node");
            });
            assert!(false, "can't spawn all hotstuff-consensus node");
            return;
        }

        // join.
        let flag = childs.into_iter().fold(true, |p, mut c| {
            p & c
                .wait()
                .expect("can't success exec hotstuff-consensus")
                .success()
        });

        assert!(flag);

        // verify test.
    }
}
