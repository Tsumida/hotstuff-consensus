//! Utility for testing.
use simplelog::{CombinedLogger, Config, LevelFilter, WriteLogger};

const DEBUG_MODE: bool = true;

pub(crate) const TEST_MYSQL_ADDR: &str =
    "mysql://root:helloworld@localhost:3306/hotstuff_test_mocker";

pub(crate) fn init_logger() {
    if DEBUG_MODE {
        let _ = CombinedLogger::init(vec![
            //TermLogger::new(LevelFilter::Debug, Config::default(), TerminalMode::Mixed),
            WriteLogger::new(
                LevelFilter::Debug,
                Config::default(),
                std::fs::File::create("./test-output/my_rust_bin.log").unwrap(),
            ),
        ]);
    }
}

pub(crate) fn init_pm_logger(log_path: &str) {
    if DEBUG_MODE {
        let _ = CombinedLogger::init(vec![WriteLogger::new(
            LevelFilter::Debug,
            Config::default(),
            std::fs::File::create(log_path).unwrap(),
        )]);
    }
}
