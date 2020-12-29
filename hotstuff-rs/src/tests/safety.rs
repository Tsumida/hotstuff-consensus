use serde::de::Expected;

use super::mocker::{ExpectedState, MockHotStuff};
use crate::safety::basic::Txn;
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
        .load_continue_chain(vec![
            Txn::new("a1".as_bytes()),
            Txn::new("a2".as_bytes()),
            Txn::new("a3".as_bytes()),
        ]);

    let expected_1 = ExpectedState::LockedInHeight(1);
    mhs.check_with_expected_state(&expected_1);

    mhs.extend_from(Txn::new("a1".as_bytes()), Txn::new("b1".as_bytes()));
    mhs.extend_from(Txn::new("b1".as_bytes()), Txn::new("b2".as_bytes()));
    mhs.check_with_expected_state(&expected_1);

    mhs.extend_from(Txn::new("b2".as_bytes()), Txn::new("b3".as_bytes()));

    let expected_2 = ExpectedState::CommittedBeforeHeight(1);
    let expected_3 = ExpectedState::LockedInHeight(2);

    mhs.check_with_expected_state(&expected_2);
    mhs.check_with_expected_state(&expected_3);

    mhs.extend_from(Txn::new("a3".as_bytes()), Txn::new("a4".as_bytes()));

    mhs.check_with_expected_state(&expected_2);
    mhs.check_with_expected_state(&expected_3);
}
