use serde::de::Expected;

use super::mocker::{ExpectedState, MockHotStuff};
#[test]
fn test_competitive_branchs() {
    //
    //  a1 <--- a2 <---- a3 <---------------------- a4
    //   |
    //   <------=-------------- b1 <----- b2 <------------- b3
    //
    // steps:
    // 1. create chain a1 <--- a2 <--- a3, now a1 is locked.
    // 2. propose b1, b2
    // 3. propose a4, commmit a1, lock a2
    // 4. propose b3, rejected

    let n = 4;
    let f = 1;
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

    mhs.extend_from(format!("b2"), format!("b3"));

    mhs.check_with_expected_state(&expected_2);
    mhs.check_with_expected_state(&expected_3);
}

#[test]
fn test_consecutive_commit(){
    // init_node <- a1 <- a2 <- a3 <- a4 <- a5 <- a6 <- a7
    //                             committed   locked
    let n = 4;
    let f = 1;
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
        format!("a4"), 
        format!("a5"), 
        format!("a6"), 
        format!("a7"), 
    ]);

    mhs.check_with_expected_state(&ExpectedState::LockedInHeight(5));
    mhs.check_with_expected_state(&ExpectedState::CommittedBeforeHeight(4));
}