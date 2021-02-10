use hotstuff_rs::msg::Context;
use pacemaker::{data::PeerEvent, pacemaker::Pacemaker};
use simplelog::*;
use std::time::Duration;

fn init_logger() {
    let _ = CombinedLogger::init(vec![TermLogger::new(
        LevelFilter::Debug,
        Config::default(),
        TerminalMode::Mixed,
    )]);
}

#[tokio::main]
async fn main() {
    init_logger();
}
