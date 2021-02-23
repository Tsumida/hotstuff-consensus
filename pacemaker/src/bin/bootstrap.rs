use simplelog::*;

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
