//! Demo
use log::info;
use simplelog::*;

mod msg;
mod safety;
mod safety_storage;
mod traits;
mod utils;

#[cfg(test)]
mod tests;

fn main() {
    let _ = CombinedLogger::init(vec![TermLogger::new(
        LevelFilter::Info,
        Config::default(),
        TerminalMode::Mixed,
    )]);

    info!("demo");
}
