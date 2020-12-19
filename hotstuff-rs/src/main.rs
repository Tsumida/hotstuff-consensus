//! Demo
use log::info;
use simplelog::*;

mod msg;
mod safety;
mod safety_storage;
mod traits;
mod utils;

fn main() {
    let _ = CombinedLogger::init(vec![TermLogger::new(
        LevelFilter::Info,
        Config::default(),
        TerminalMode::Mixed,
    )]);

    info!("demo");
}
