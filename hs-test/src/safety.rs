//! MockerHotstuff depends on hss and hss depends on `SafetyStorage` trait in hotstuff-rs.
//! So tests are separated from hotstuff-rs in order to prevent circular dependencies.

#[cfg(test)]
mod test {
    use crate::{
        mocker_hotstuff::{ExpectedState, MockEvent, MockHotStuff},
        utils::init_logger,
    };
    use hotstuff_rs::safety::machine::{Ready, SafetyErr, SafetyEvent};
    use hs_data::msg::Context;

    #[test]
    fn test_competitive_branchs() {
        //
        //  init <---- a1 <--- a2 <---- a3 <---------------------- a4
        //              |
        //              <--------------------- b1 <----- b2
        //
        // steps:
        // 1. create chain a1 <--- a2 <--- a3, now a1 is locked.
        // 2. propose b1, b2
        // 3. propose a4, commmit a1, lock a2

        init_logger("./test-output/competitive_branchs.log");

        let n = 4;
        let leader = 0;
        let testee = 1;
        let mut mhs = MockHotStuff::new(n);

        mhs.specify_leader(leader).specify_testee(testee).init();

        let v = vec![format!("a1"), format!("a2"), format!("a3")];
        mhs.testee_load_consecutive_proposals(v);

        let expected_1 = ExpectedState::LockedAt(format!("a1"));
        mhs.check_hotstuff_state_with(&expected_1);

        mhs.extend_from(format!("a1"), format!("b1"));
        mhs.extend_from(format!("b1"), format!("b2"));

        mhs.check_hotstuff_state_with(&expected_1);

        mhs.extend_from(format!("a3"), format!("a4"));

        let expected_2 = ExpectedState::CommittedBeforeHeight(1);
        let expected_3 = ExpectedState::LockedAt(format!("a2"));

        mhs.check_hotstuff_state_with(&expected_2);
        mhs.check_hotstuff_state_with(&expected_3);

        // if we propose b3 based on b2, there is no way to form a quorum certificate of b2.
        // but if we mistakenly create b3 with qc of b2 and propose it,
        // the testee may switch to branch b2 even if it's locked at a2 (undefined behavior).
    }

    #[test]
    fn test_consecutive_commit() {
        //
        // init <--- a1 <--- a2 <--- a3 <--- a4
        //        committed  locked

        let n = 4;
        let leader = 0;
        let testee = 1;
        let mut mhs = MockHotStuff::new(n);

        init_logger("./test-output/consecutive_commit.log");

        mhs.specify_leader(leader).specify_testee(testee).init();

        mhs.testee_load_consecutive_proposals(vec![
            format!("a1"),
            format!("a2"),
            format!("a3"),
            format!("a4"),
        ]);

        mhs.check_hotstuff_state_with(&ExpectedState::LockedAt(format!("a2")));
        mhs.check_hotstuff_state_with(&ExpectedState::CommittedBeforeHeight(1));
    }

    #[test]
    fn test_propose_with_corrupted_qc() {
        //
        //              |       corrupted        |
        // init <--- a1 <--- a2 <--- a3 <--- a4
        //              <--- b1 <--- b2 <--- b3
        //              |        correct         |
        //
        // Machine should validate qc independently.
        // all of a2, a3, a4 should be rejected due to corrupted qc.
        // And then propose correct branch b1 <- b2 <- b3
        // Now b1 is locked.

        let n = 4;
        let leader = 0;
        let testee = 1;
        let adversial = leader;
        let mut mhs = MockHotStuff::new(n);

        init_logger("./test-output/propose_with_corrupted_qc.log");

        mhs.specify_leader(leader)
            .specify_testee(testee)
            .specify_adversial(adversial)
            .init();

        mhs.testee_load_consecutive_proposals(vec![format!("a1")]);

        mhs.propose_with_corrupted_qc(format!("a1"), format!("a2"));
        mhs.propose_with_corrupted_qc(format!("a2"), format!("a3"));
        mhs.propose_with_corrupted_qc(format!("a3"), format!("a4"));

        mhs.check_hotstuff_state_with(&ExpectedState::CommittedBeforeHeight(0));
        mhs.check_hotstuff_state_with(&ExpectedState::LockedAt(format!("init")));

        mhs.extend_from(format!("a1"), format!("b1"));
        mhs.extend_from(format!("b1"), format!("b2"));
        mhs.extend_from(format!("b2"), format!("b3"));

        mhs.check_hotstuff_state_with(&ExpectedState::CommittedBeforeHeight(1));
        mhs.check_hotstuff_state_with(&ExpectedState::LockedAt(format!("b1")));
    }

    #[test]
    fn test_new_proposal() {
        //
        //                              qc_high
        //                               |
        // init <- a1 <- a2 <- a3 <- a4 <- a5
        //               |                  |
        //              locked             new-proposal without qc
        // Received new-view msg respectively based on a1, a2, a3, a4,
        // leader should make new proposal base on a4.

        let n = 4;
        let leader = 0;
        let testee = 0;
        let mut mhs = MockHotStuff::new(n);

        init_logger("./test-output/new_proposal.log");

        mhs.specify_leader(leader).specify_testee(testee).init();

        mhs.testee_load_consecutive_proposals(vec![
            format!("a1"),
            format!("a2"),
            format!("a3"),
            format!("a4"),
            format!("a5"), // recv a5
        ]);

        // recv new-view msgs a2, a3, a4, a5,
        let output = mhs
            .testee_recv_new_view_msgs(vec![
                (0, format!("a2")), // qc of a1
                (1, format!("a3")),
                (2, format!("a4")), // qc of a3
            ])
            .testee_make_proposal(format!("a6"));

        // leader's qc_high is qc of a4 => new proposal is based on q4
        if let Ready::NewProposal(_, prop, ..) = output {
            mhs.check_proposal_with(&ExpectedState::QcOf(format!("a4"), &prop))
                .check_proposal_with(&ExpectedState::ParentIs(format!("a4"), &prop));
        } else {
            panic!();
        }
    }

    #[test]
    fn test_corrupted_new_view_msg() {
        //
        //                           corrupted qc
        // init <- a1 <- a2 <- a3 <--------------- a4      a5
        //                     |                            |
        //                     |<---------------------------|
        //                            correct qc of a3
        // recv corrupted new view msgs. Machine must validate NewView Msg before further processing.
        let n = 4;
        let leader = 0;
        let testee = 0;
        let adversial = 1;
        let mut mhs = MockHotStuff::new(n);

        init_logger("./test-output/corrupted_new_view_msg.log");

        mhs.specify_leader(leader)
            .specify_testee(testee)
            .specify_adversial(adversial)
            .init();

        mhs.testee_load_consecutive_proposals(vec![
            format!("a1"),
            format!("a2"),
            format!("a3"),
            format!("a4"), // we have correct qcv of a3 now
        ]);

        let output = mhs
            // send NewView Msg with corrupted qc of a3
            .testee_recv_corrupted_view_msg(format!("a3"))
            // propose a5, based on a3
            .testee_make_proposal(format!("a5"));

        match output {
            Ready::NewProposal(_, prop, ..) => {
                mhs.check_proposal_with(&ExpectedState::QcOf(format!("a3"), &prop))
                    .check_proposal_with(&ExpectedState::ParentIs(format!("a3"), &prop));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_corrupted_vote() {
        //
        // init <-- a1 <--- a2 <--- a3
        //                   ↑
        //                   ------ a4
        //
        // Machine should validate vote independently.
        // Steps:
        // 1. Leader propose a4 based on a2 and recv 2 vote already (leader inclueded).
        // 2. Leader recv a corrupted vote and reject it.
        // 3. Leader recv a correct vote and then form qc.

        let n = 4;
        let leader = 0;
        let testee = 0;
        let adversial = 1;
        let mut mhs = MockHotStuff::new(n);

        init_logger("./test-output/corrupted_vote.log");

        mhs.specify_leader(leader)
            .specify_testee(testee)
            .specify_adversial(adversial)
            .init();

        mhs.testee_load_consecutive_proposals(vec![format!("a1"), format!("a2"), format!("a3")]);

        // new leader propose a4 and sign to it!
        mhs.testee_make_proposal(format!("a4"));

        mhs.check_hotstuff_state_with(&ExpectedState::QcHighOf(format!("a2")));

        mhs.testee_recv_votes(vec![
            MockEvent::AcceptedVote(2, format!("a4")),
            MockEvent::CorruptedVote(adversial, format!("a4")),
        ]);

        mhs.check_hotstuff_state_with(&ExpectedState::QcHighOf(format!("a2")));

        // recv 3 vote and form qc.
        mhs.testee_recv_votes(vec![MockEvent::AcceptedVote(3, format!("a4"))]);

        // if qc of a3 is formed, a3 will be new leaf.
        mhs.check_hotstuff_state_with(&ExpectedState::QcHighOf(format!("a4")));
    }

    #[test]
    fn test_sync_state() {
        //
        // init <- a1 <- a2 <- a3
        //                                                  <- a8  reject
        // branch sync            <- a4 <- a5
        // branch sync                         <- a6 <- a7
        //                                                  <- a8  accept

        // Steps:
        // 1. The testee takes a1, a2, a3.
        // 2. The testee got a6 based on a5, because of lack of a4, a5, testee consider a6 is corrupted and rejest it.
        // 3. The testee got a4 <- a5 <- a6 <- a7.
        // 4. The testee got a8, accept it, locked at a6 and committed a5

        let n = 4;
        let leader = 0;
        let testee = 0;
        let adversial = 1;
        let mut mhs = MockHotStuff::new(n);

        init_logger("./test-output/sync_state.log");

        mhs.specify_leader(leader)
            .specify_testee(testee)
            .specify_adversial(adversial)
            .init();

        mhs.testee_load_consecutive_proposals(vec![format!("a1"), format!("a2"), format!("a3")]);

        let branch = mhs.prepare_proposals(&vec![
            format!("a4"),
            format!("a5"),
            format!("a6"),
            format!("a7"),
            format!("a8"),
        ]);

        let (a6, _) = branch.last().unwrap();

        let output = mhs.testee().process_safety_event(SafetyEvent::RecvProposal(
            Context::single(
                format!("replica-{}", leader),
                format!("replica-{}", testee),
                a6.height(),
            ),
            std::sync::Arc::new(a6.as_ref().clone()),
        ));

        match output {
            Err(SafetyErr::CorruptedQC) => {}
            _ => panic!(),
        }

        mhs.check_hotstuff_state_with(&ExpectedState::LockedAt(format!("a1")));

        // sync a4 and a5
        mhs.sync_state(branch.iter().map(|(node, _)| node.as_ref().clone()).take(2));
        mhs.sync_state(
            branch[2..4]
                .into_iter()
                .map(|(node, _)| node.as_ref().clone()),
        );

        // propose a6
        mhs.extend_from(format!("a7"), format!("a8"));

        mhs.check_hotstuff_state_with(&ExpectedState::LockedAt(format!("a6")));
    }
}
