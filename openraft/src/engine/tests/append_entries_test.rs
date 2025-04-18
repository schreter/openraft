use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::core::ServerState;
use crate::engine::testing::log_id;
use crate::engine::testing::UTConfig;
use crate::engine::Command;
use crate::engine::Condition;
use crate::engine::Engine;
use crate::entry::RaftEntry;
use crate::error::RejectAppendEntries;
use crate::raft_state::IOId;
use crate::raft_state::LogStateReader;
use crate::testing::blank_ent;
use crate::type_config::TypeConfigExt;
use crate::utime::Leased;
use crate::vote::raft_vote::RaftVoteExt;
use crate::EffectiveMembership;
use crate::Entry;
use crate::Membership;
use crate::MembershipState;
use crate::Vote;

fn m01() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {0,1}], [])
}

fn m23() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {2,3}], [])
}

fn m34() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {3,4}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 2;
    eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::from_millis(500), Vote::new(2, 1));
    eng.state.log_ids.append(log_id(1, 1, 1));
    eng.state.log_ids.append(log_id(2, 1, 3));
    eng.state.apply_progress_mut().accept(log_id(0, 1, 0));
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
    );
    eng.state.server_state = eng.calc_server_state();
    eng
}

#[test]
fn test_append_entries_vote_is_rejected() -> anyhow::Result<()> {
    let mut eng = eng();

    let res = eng.append_entries(&Vote::new(1, 1), None, Vec::<Entry<UTConfig>>::new());

    assert_eq!(Err(RejectAppendEntries::ByVote(Vote::new(2, 1))), res);
    assert_eq!(
        &[
            log_id(1, 1, 1), //
            log_id(2, 1, 3),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(2, 1, 3)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_append_entries_prev_log_id_is_applied() -> anyhow::Result<()> {
    // An applied log id has to be committed thus
    let mut eng = eng();
    eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::from_millis(500), Vote::new(1, 2));
    eng.output.take_commands();

    let res = eng.append_entries(
        &Vote::new_committed(2, 1),
        Some(log_id(0, 1, 0)),
        Vec::<Entry<UTConfig>>::new(),
    );

    assert_eq!(Ok(()), res);
    assert_eq!(
        &[
            log_id(1, 1, 1), //
            log_id(2, 1, 3), //
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(2, 1, 3)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new_committed(2, 1)
            },
            Command::UpdateIOProgress {
                when: Some(Condition::IOFlushed {
                    io_id: IOId::new_log_io(Vote::new(2, 1).into_committed(), None)
                }),
                io_id: IOId::new_log_io(Vote::new(2, 1).into_committed(), Some(log_id(0, 1, 0)))
            }
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_append_entries_prev_log_id_conflict() -> anyhow::Result<()> {
    let mut eng = eng();

    let res = eng.append_entries(
        &Vote::new_committed(2, 1),
        Some(log_id(2, 1, 2)),
        Vec::<Entry<UTConfig>>::new(),
    );

    assert_eq!(
        Err(RejectAppendEntries::ByConflictingLogId {
            expect: log_id(2, 1, 2),
            local: Some(log_id(1, 1, 2)),
        }),
        res
    );
    assert_eq!(
        &[
            log_id(1, 1, 1), //
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(1, 1, 1)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);
    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new_committed(2, 1)
            },
            Command::TruncateLog { since: log_id(1, 1, 2) },
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_append_entries_prev_log_id_is_committed() -> anyhow::Result<()> {
    let mut eng = eng();

    let res = eng.append_entries(&Vote::new_committed(2, 1), Some(log_id(0, 1, 0)), vec![
        blank_ent(1, 1, 1),
        blank_ent(2, 1, 2),
    ]);

    assert_eq!(Ok(()), res);
    assert_eq!(
        &[
            log_id(1, 1, 1), //
            log_id(2, 1, 2), //
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(2, 1, 2)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);
    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new_committed(2, 1)
            },
            Command::TruncateLog { since: log_id(1, 1, 2) },
            Command::AppendInputEntries {
                committed_vote: Vote::new(2, 1).into_committed(),
                entries: vec![blank_ent(2, 1, 2)]
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_append_entries_prev_log_id_not_exists() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::from_millis(500), Vote::new(1, 2));
    eng.output.take_commands();

    let res = eng.append_entries(&Vote::new_committed(2, 1), Some(log_id(2, 1, 4)), vec![
        blank_ent(2, 1, 5),
        blank_ent(2, 1, 6),
    ]);

    assert_eq!(
        Err(RejectAppendEntries::ByConflictingLogId {
            expect: log_id(2, 1, 4),
            local: None,
        }),
        res
    );
    assert_eq!(
        &[
            log_id(1, 1, 1), //
            log_id(2, 1, 3), //
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(2, 1, 3)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(
        vec![Command::SaveVote {
            vote: Vote::new_committed(2, 1)
        },],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_append_entries_conflict() -> anyhow::Result<()> {
    // prev_log_id matches,
    // The second entry in entries conflict.
    // This request will replace the effective membership.
    // committed is greater than entries.
    // It is no longer a member, change to learner
    let mut eng = eng();

    let resp = eng.append_entries(&Vote::new_committed(2, 1), Some(log_id(1, 1, 1)), vec![
        blank_ent(1, 1, 2),
        Entry::new_membership(log_id(3, 1, 3), m34()),
    ]);

    assert_eq!(Ok(()), resp);
    assert_eq!(
        &[
            log_id(1, 1, 1), //
            log_id(3, 1, 3), //
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(3, 1, 3)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(3, 1, 3)), m34())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);
    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new_committed(2, 1)
            },
            Command::TruncateLog { since: log_id(2, 1, 3) },
            Command::AppendInputEntries {
                committed_vote: Vote::new(2, 1).into_committed(),
                entries: vec![Entry::new_membership(log_id(3, 1, 3), m34())]
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}
