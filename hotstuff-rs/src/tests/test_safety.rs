use std::unreachable;

use super::mocker::{ExpectedState, MockHotStuff, init_logger};
use crate::safety::machine::Ready; 

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

    // if we propose b3 based on b2, there is no way to form a quorum certificate of b2. 
    // but if we mistakenly create b3 with qc of b2 and propose it, 
    // the testee may switch to branch b2 even if it's locked at a2 (undefined behavior). 
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
fn test_corrupted_qc(){
    //             |   corrupted   |
    // init <- a1 <- a2 <- a3 <- a4
    //            <- b1 <- b2 <- b3
    //             |   correct     |
    // Machine should validate qc independently. 
    // all of a2, a3, a4 should be rejected due to corrupted qc. 
    // And then propose correct branch b1 <- b2 <- b3
    // note that b1.height = 5

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

    mhs.propose_with_corrupted_qc(format!("a1"), format!("a2"));
    mhs.propose_with_corrupted_qc(format!("a2"), format!("a3"));
    mhs.propose_with_corrupted_qc(format!("a3"), format!("a4"));

    mhs.check_with_expected_state(&ExpectedState::CommittedBeforeHeight(0)); 
    mhs.check_with_expected_state(&ExpectedState::LockedInHeight(0));

    mhs.extend_from(format!("a1"), format!("b1"));
    mhs.extend_from(format!("b1"), format!("b2"));
    mhs.extend_from(format!("b2"), format!("b3"));

    mhs.check_with_expected_state(&ExpectedState::CommittedBeforeHeight(1)); 
    mhs.check_with_expected_state(&ExpectedState::LockedInHeight(5));
}

#[test]
fn test_new_proposal(){
    // init <- a1 <- a2 <- a3 <- a4 <- a5
    //               |                  |
    //              locked             new-proposal without qc
    // Received new-view msg respectively based on a2, a3, a4, a5, 
    // leader should make new proposal base on a5. 

    let n = 4;
    let leader = 0;
    let testee = 0;
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
        format!("a5"), 
    ]);

    // recv new-view msgs a2, a3, a4, a5,
    let output = mhs
    .recv_new_view_msg(
        n, 
        vec![format!("a2"), 
                format!("a3"), 
                format!("a4"), 
                format!("a5")])
    .make_proposal(format!("a6")); 

    if let Ready::NewProposal(_, prop, _) = output{
        mhs.check_proposal_with(
            &ExpectedState::PropsalBaseOn(format!("a4"), &prop), 
        );
    }else{
        unreachable!()
    }
}

#[test]
#[ignore = "unimplemented"]
fn test_corrupted_new_view_msg(){
    // init <- a1 
    // 
    // recv corrupted new view msgs. Machine must validate NewView Msg before further processing. 
}

#[test]
#[ignore = "unimplemented"]
fn test_corrupted_vote(){
    // init <- a1 <- a2 
    //              
    // Machine should validate vote independently. 

    let n = 4;
    let leader = 0;
    let testee = 0;
    let mut mhs = MockHotStuff::new(n);

    init_logger();

    mhs.specify_leader(leader)
        .specify_testee(testee)
        .init();

    mhs.load_continue_chain(vec![
        format!("a1"), 
        format!("a2"), 
    ]);
    // TODO: 
}

#[test]
#[ignore = "unimplemented"]
fn test_commit_failed_proposal(){
    // init <- a1 <- a2 (failed) <- a3 <- a4 <- a5 <- a6
    //  
    // Leader failed to form a QC for a2 at view 2. 
    // once recv 3 qc of a3, replica will commit    
    
}

