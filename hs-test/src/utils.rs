//! Utility for testing.
use simplelog::{CombinedLogger, Config, ConfigBuilder, LevelFilter, WriteLogger};

const DEBUG_MODE: bool = true;

pub(crate) const TEST_MYSQL_ADDR: &str =
    "mysql://root:helloworld@localhost:3306/hotstuff_test_mocker";

pub(crate) const MYSQL_ADDR_CUSTOM_DB: &str =
    "mysql://root:helloworld@localhost:3306/hotstuff_test_{}";

pub(crate) fn init_logger(log_path: &str) {
    if DEBUG_MODE {
        let path = std::path::Path::new(log_path);
        let parent_dir_path = path.parent().unwrap();
        std::fs::create_dir_all(parent_dir_path).unwrap();
        let _ = CombinedLogger::init(vec![
            //TermLogger::new(LevelFilter::Debug, Config::default(), TerminalMode::Mixed),
            WriteLogger::new(
                LevelFilter::Debug,
                // filter sqlx logs.
                ConfigBuilder::new()
                    .add_filter_ignore(format!("sqlx"))
                    .build(),
                std::fs::File::create(path).unwrap(), //std::fs::File::create(log_path).unwrap(),
            ),
        ]);
    }
}
