use serde::de::Expected;
use simplelog::CombinedLogger;

use super::mocker::{ExpectedState, MockHotStuff};
use simplelog::*; 

fn init_logger(){
    let _ = CombinedLogger::init(
        vec![
            //TermLogger::new(LevelFilter::Debug, Config::default(), TerminalMode::Mixed),
            //WriteLogger::new(LevelFilter::Debug, Config::default(), std::fs::File::create("./my_rust_bin.log").unwrap())
        ]
    );
}

#[test]
fn test_competitive_branchs() {
    //
    //  a1 <--- a2 <---- a3 <---------------------- a4
    //   |
    //   <--------------------- b1 <----- b2 
    //
    // steps:
    // 1. create chain a1 <--- a2 <--- a3, now a1 is locked.
    // 2. propose b1, b2
    // 3. propose a4, commmit a1, lock a2

    init_logger();

    let n = 4;
    let leader = 0;
    let testee = 1;
    let mut mhs = MockHotStuff::new(n);

    mhs.specify_leader(leader)
        .specify_testee(testee)
        .init();
    
    mhs.load_continue_chain(vec![
        format!("a1"), 
        format!("a2"),
        format!("a3"),
    ]);
    
    let expected_1 = ExpectedState::LockedInHeight(1);
    mhs.check_with_expected_state(&expected_1);

    mhs.extend_from(format!("a1"), format!("b1"));
    mhs.extend_from(format!("b1"), format!("b2"));

    mhs.check_with_expected_state(&expected_1);

    mhs.extend_from(format!("a3"), format!("a4"));

    let expected_2 = ExpectedState::CommittedBeforeHeight(1);
    let expected_3 = ExpectedState::LockedInHeight(2);

    mhs.check_with_expected_state(&expected_2);
    mhs.check_with_expected_state(&expected_3);
    // if we propose b3 based on b2, there is no way to form quorum certificate. 
}

#[test]
fn test_consecutive_commit(){
    // init_node <-- a1 <-- a2 <-- a3 <-- a4
    //          committed  locked
    let n = 4;
    let leader = 0;
    let testee = 1;
    let mut mhs = MockHotStuff::new(n);

    init_logger();

    mhs.specify_leader(leader)
        .specify_testee(testee)
        .init();

    mhs.load_continue_chain(vec![
        format!("a1"), 
        format!("a2"), 
        format!("a3"), 
        format!("a4"), 
    ]);

    mhs.check_with_expected_state(&ExpectedState::LockedInHeight(2));
    mhs.check_with_expected_state(&ExpectedState::CommittedBeforeHeight(1));
}


#[test]
#[ignore= "unimplemented"]
fn test_corrupted_qc(){
    // init <- a1 <- a2 (withcorrupted qc for a1)
    // Machine should validate qc independently. 
    let n = 4;
    let leader = 0;
    let testee = 1;
    let mut mhs = MockHotStuff::new(n);

    init_logger();

    mhs.specify_leader(leader)
        .specify_testee(testee)
        .init();

    mhs.load_continue_chain(vec![
        format!("a1"), 
    ]);

}