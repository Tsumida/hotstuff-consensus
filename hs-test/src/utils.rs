//! Utility for testing.
use simplelog::{CombinedLogger, Config, LevelFilter, WriteLogger};

const DEBUG_MODE: bool = false;

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
