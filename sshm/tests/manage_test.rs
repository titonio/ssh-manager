//! The manage frame's interaction, driven as data (#36).
//!
//! Every acceptance criterion about *which key means what* is proved here, with
//! no terminal and no PTY, because the whole interaction is
//! [`manage::step`]: `(state, key, selection) → (next state, effects)`.
//!
//! The tests are written against the ticket's worked example —
//! `◆ Delete [prod] web-01? (y/N)` over the same Connection the parent spec
//! (#31) uses for its settle trace — so the expectations come from the spec
//! rather than from the code under test.
//!
//! The dangerous cases get the most attention, because a delete confirm that
//! deletes the wrong thing is the worst failure this surface can have:
//!
//! * a non-`y` answer must not delete;
//! * `y` must delete exactly one Connection;
//! * the delete must be scoped to the Connection selected when the chord was
//!   pressed, not to wherever the cursor drifted to afterwards.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sshm::config::Connection;
use sshm::inline::InlineOutcome;
use sshm::manage::{
    self, step, AddField, AddSequence, DeleteOutcome, EditOutcome, Effect, ManageState, Phase,
    Trace,
};

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures
// ─────────────────────────────────────────────────────────────────────────────

/// The Connection the ticket names in its confirm string.
fn web01() -> Connection {
    Connection {
        id: "id-web-01".into(),
        alias: "web-01".into(),
        host: "10.0.0.4".into(),
        user: "deploy".into(),
        port: 22,
        key_path: None,
        folder: Some("prod".into()),
    }
}

/// A second Connection, so "exactly one" and "not the neighbour" are provable.
fn web02() -> Connection {
    Connection {
        id: "id-web-02".into(),
        alias: "web-02".into(),
        host: "10.0.0.5".into(),
        user: "deploy".into(),
        port: 22,
        key_path: None,
        folder: Some("prod".into()),
    }
}

fn ctrl(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
}

fn plain(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

// ─────────────────────────────────────────────────────────────────────────────
// Slice 5 — the list step: filter, cancel, and the add/edit routes
// ─────────────────────────────────────────────────────────────────────────────

/// User story 21, stated as the sweep it has to be: not one printable ASCII
/// character starts a management action. `a`, `e` and `x` in particular must
/// be typeable — they are letters a user searches with before they are chord
/// letters.
#[test]
fn no_printable_character_starts_a_management_action() {
    for ch in (0x20u8..0x7f).map(|b| b as char) {
        let step = step(&ManageState::new(), plain(ch), Some(&web01()));

        assert_eq!(
            step.state.phase,
            Phase::List,
            "{ch:?} must not open a confirm or any other step"
        );
        assert!(
            !step.effects.iter().any(|e| matches!(
                e,
                Effect::Delete { .. } | Effect::Add { .. } | Effect::Exit(_)
            )),
            "{ch:?} must not ask the driver to change the Connection set or \
             leave the frame: {:?}",
            step.effects
        );
    }
}

/// The three letters the management chords are spelled with are exactly the
/// ones most at risk of being stolen from the filter, so they are named here
/// rather than left to the sweep above.
#[test]
fn the_chord_letters_are_still_search_text() {
    for ch in ['a', 'e', 'x'] {
        let step = step(&ManageState::new(), plain(ch), Some(&web01()));

        assert_eq!(
            step.state.query,
            ch.to_string(),
            "`{ch}` must type into the filter, not fire the Ctrl+{ch} chord"
        );
        assert!(
            step.effects.is_empty(),
            "`{ch}` alone must ask for nothing: {:?}",
            step.effects
        );
    }
}

/// User story 21, taken literally: the manage frame's filter accepts
/// **every printable character**, and `j` and `k` are printable. They were
/// bound to movement by the pick path and carried over here, which made
/// `jakarta`, `kjell` and every other search containing them untypeable —
/// a filter with two holes is not a live filter.
///
/// Movement in the manage frame is the arrow keys. The rail says `↑↓
/// navigate` and nothing else, so nothing is advertised that does not work
/// and nothing is stolen that the user needs for typing.
#[test]
fn j_and_k_type_into_the_filter_rather_than_moving_the_cursor() {
    let mut state = ManageState::new();

    for ch in "jakarta".chars() {
        state = step(&state, plain(ch), Some(&web01())).state;
    }

    assert_eq!(
        state.query, "jakarta",
        "every character of the word must reach the filter"
    );
    assert_eq!(
        state.selection, 0,
        "and none of them may have moved the cursor: {:?}",
        state
    );

    let k = step(&ManageState::new(), plain('k'), Some(&web01()));
    assert_eq!(k.state.query, "k", "`k` is filter text too");
    assert_eq!(k.state.selection, 0, "`k` must not move the cursor");
}

/// Arrows are the manage frame's movement, and the only thing that is.
#[test]
fn arrows_move_the_cursor_in_the_manage_frame() {
    let down = step(&ManageState::new(), key(KeyCode::Down), Some(&web01()));
    assert_eq!(down.state.selection, 1);

    let up = step(&down.state, key(KeyCode::Up), Some(&web01()));
    assert_eq!(up.state.selection, 0);

    let clamped = step(&ManageState::new(), key(KeyCode::Up), Some(&web01()));
    assert_eq!(clamped.state.selection, 0, "up at the top stays at the top");
}

/// The whole printable range, asserted as filter text rather than movement:
/// story 21 is about the set, not about three example letters.
#[test]
fn every_printable_character_lands_in_the_filter() {
    for ch in (0x20u8..0x7f).map(|b| b as char) {
        let step = step(&ManageState::new(), plain(ch), Some(&web01()));

        assert_eq!(
            step.state.query,
            ch.to_string(),
            "{ch:?} must reach the query, not be spent on movement or a chord"
        );
        assert_eq!(
            step.state.selection, 0,
            "{ch:?} must not move the cursor in the manage frame"
        );
    }
}

#[test]
fn ctrl_c_cancels_the_frame() {
    let step = step(&ManageState::with_query("web"), ctrl('c'), Some(&web01()));

    assert_eq!(step.effects, vec![Effect::Exit(InlineOutcome::Cancelled)]);
}

#[test]
fn ctrl_c_at_an_open_confirm_cancels_without_deleting() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));

    let step = step(&armed.state, ctrl('c'), Some(&web01()));

    assert_eq!(
        step.effects,
        vec![Effect::Exit(InlineOutcome::Cancelled)],
        "Ctrl+C leaves; it must not settle the confirm as a yes on the way out"
    );
}

/// `Ctrl+E` no longer leaves the frame.
///
/// It used to exit with the selection on the edit route — byte-identical
/// to Enter, which is why the hint rail refused to advertise it. #37 gave
/// the chord its own destination: the in-place editor, which stays on the
/// glass. This test exists to pin that the old exit is *gone*, because a
/// chord that both opens an editor and exits the frame would be two
/// actions on one keystroke.
#[test]
fn ctrl_e_no_longer_exits_the_frame() {
    let step = step(&ManageState::new(), ctrl('e'), Some(&web01()));

    assert!(
        !step.effects.iter().any(|e| matches!(e, Effect::Exit(_))),
        "Ctrl+E must stay on the glass and open the editor, not exit: {:?}",
        step.effects
    );
}

#[test]
fn ctrl_e_with_nothing_selected_routes_nothing() {
    let step = step(&ManageState::new(), ctrl('e'), None);

    assert!(
        step.effects.is_empty(),
        "with no row under the cursor the frame must not invent a target to edit"
    );
    assert_eq!(step.state, ManageState::new());
}

/// `Ctrl+A` opens the Clack step-sequence the ticket names — `◆ Alias` →
/// `◆ Host` → `◆ Port` → `◆ Key` → `◆ Folder` — at its first step (#37).
///
/// Nothing is asked of the driver yet: opening a question is not an
/// effect. The sequence is pure state until the last step is answered.
#[test]
fn ctrl_a_opens_the_add_sequence_at_the_alias_step() {
    let step = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let sequence = match &step.state.phase {
        Phase::Add(sequence) => sequence,
        other => panic!("Ctrl+A must open the add sequence, got {other:?}"),
    };

    assert_eq!(
        sequence.field,
        AddField::Alias,
        "the sequence starts at Alias, the first of the spec's five"
    );
    assert!(
        sequence.input.is_empty(),
        "and starts with nothing typed: {sequence:?}"
    );
    assert!(
        step.effects.is_empty(),
        "opening a step must ask the driver for nothing: {:?}",
        step.effects
    );
}

/// The add sequence is reachable from the list whatever the list is doing,
/// and takes the frame off the list without leaving the frame.
#[test]
fn ctrl_a_opens_the_sequence_from_a_live_filter_without_losing_it() {
    let filtered = ManageState::with_query("web");

    let step = step(&filtered, ctrl('a'), Some(&web01()));

    assert!(matches!(step.state.phase, Phase::Add(_)));
    assert_eq!(
        step.state.query, "web",
        "the filter is the user's; the sequence borrows the frame, not the query"
    );
}

/// A new action clears the previous action's note, so the frame never shows
/// a stale result above a list that has since moved on.
#[test]
fn a_new_action_clears_the_previous_note() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));
    let answered = step(&armed.state, plain('y'), Some(&web01()));
    let deleted = manage::settle_delete(&answered.state, &web01(), DeleteOutcome::Removed(web01()))
        .expect("a real removal settles onto the list");
    assert_eq!(
        deleted.trace,
        Some(Trace::Deleted {
            connection: web01()
        })
    );

    let next = step(&deleted, ctrl('a'), Some(&web02()));

    assert_eq!(
        next.state.trace, None,
        "the delete note must not survive on top of a different action"
    );
}

/// The worst failure this surface can have: `Ctrl+X` on one Connection, the
/// cursor moves, and a different Connection is deleted. The confirm captures
/// its target by value, so the answer is about the captured one no matter
/// what the caller's selection says when the answer arrives.
#[test]
fn the_delete_is_scoped_to_the_connection_under_the_chord_not_the_cursor() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));

    // The caller now reports a different selection: the cursor drifted, or
    // the list re-filtered out from under it. Either way the confirm was
    // asked about web-01, so web-01 is what `y` answers about.
    let step = step(&armed.state, plain('y'), Some(&web02()));

    assert_eq!(
        step.effects,
        vec![Effect::Delete { target: web01() }],
        "the delete must name the Connection the confirm was raised on"
    );
    assert_eq!(
        step.state.phase,
        Phase::List,
        "and the frame must come back to the list, still answering about that one"
    );
}

/// Movement is inert while a confirm is open, so the cursor cannot drift at
/// all — the scoping is belt-and-braces, not the only defence.
///
/// `j`/`k` are not in this list on purpose: they are printable, and a
/// printable at the confirm declines and filters (see
/// `a_stray_printable_at_the_confirm_declines_and_filters`). Treating them as
/// movement here would be testing the wrong rule.
#[test]
fn movement_does_not_happen_while_the_confirm_is_open() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));

    for code in [
        KeyCode::Down,
        KeyCode::Up,
        KeyCode::Enter,
        KeyCode::Backspace,
    ] {
        let step = step(&armed.state, key(code), Some(&web01()));

        assert_eq!(
            step.state, armed.state,
            "{code:?} must leave the confirm exactly as it was: the target is \
             fixed the moment the chord is pressed"
        );
        assert!(step.effects.is_empty(), "{code:?} must ask for nothing");
    }
}

#[test]
fn answering_y_asks_for_exactly_one_delete_and_returns_to_the_list() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));

    let step = step(&armed.state, plain('y'), Some(&web01()));

    assert_eq!(
        step.effects,
        vec![Effect::Delete { target: web01() }],
        "`y` must ask for exactly one delete, of the Connection the confirm named"
    );
    assert_eq!(
        step.state.phase,
        Phase::List,
        "the confirm is over once it has been answered"
    );
    assert_eq!(
        step.state.trace, None,
        "the keystroke must not pre-commit a `deleted` note. The state machine \
         owns no disk: the claim is earned by the store's answer, folded in by \
         `settle_delete`"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Slice 6 — the note reports what the store actually did
// ─────────────────────────────────────────────────────────────────────────────

/// `Store::remove` returning `Ok(None)` means *nothing was deleted and
/// nothing was written*. A frame that answers that with `◇ deleted` is
/// lying about the user's `connections.json`: the Connection is still
/// there, and the next `sshm manage` will show it again.
///
/// This is the seam where the claim and the fact meet. The state machine
/// asks for the delete; the driver runs it; `settle_delete` is the only
/// place that decides what the frame is allowed to say afterwards.
#[test]
fn a_delete_that_removed_nothing_does_not_claim_a_deletion() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));
    let answered = step(&armed.state, plain('y'), Some(&web01()));

    let state = manage::settle_delete(&answered.state, &web01(), DeleteOutcome::Absent)
        .expect("a delete that found nothing is not a failure: the frame stays up");

    assert!(
        !matches!(state.trace, Some(Trace::Deleted { .. })),
        "the frame may not claim a deletion the store never performed: {:?}",
        state.trace
    );
    assert_eq!(
        state.trace,
        Some(Trace::DeleteFailed {
            connection: web01()
        }),
        "it must instead say that the delete did not happen"
    );
    assert_eq!(
        state.phase,
        Phase::List,
        "and it must stay on the list rather than abandon or collapse the frame"
    );
}

/// The mirror case: a real removal earns the `◇ deleted` note, and the note
/// names the Connection the store handed back — the same one the confirm
/// was raised on, because that is the id that was deleted.
#[test]
fn a_delete_that_removed_a_connection_earns_the_deleted_note() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));
    let answered = step(&armed.state, plain('y'), Some(&web01()));

    let state = manage::settle_delete(&answered.state, &web01(), DeleteOutcome::Removed(web01()))
        .expect("a delete that removed a Connection is the success path");

    assert_eq!(
        state.trace,
        Some(Trace::Deleted {
            connection: web01()
        })
    );
}

/// A store that refused the delete is not a note on a live frame: the list
/// it is showing can no longer be trusted. #31's `Settled` set has an
/// `error` arm for exactly this, and story 34 requires it be designed.
#[test]
fn a_store_failure_is_reported_as_a_failure_not_a_note() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));
    let answered = step(&armed.state, plain('y'), Some(&web01()));

    let outcome = manage::settle_delete(
        &answered.state,
        &web01(),
        DeleteOutcome::Failed("Permission denied (os error 13)".into()),
    );

    assert_eq!(
        outcome,
        Err("Permission denied (os error 13)".into()),
        "the store's own message must survive to be reported, not be swallowed"
    );
}

/// The classification of `Store::remove`'s three shapes is the whole
/// contract between the driver and this module, so it is pinned here rather
/// than left to be re-derived at the call site.
#[test]
fn the_store_answer_classifies_into_the_three_outcomes() {
    assert_eq!(
        DeleteOutcome::from_remove(Ok(Some(web01()))),
        DeleteOutcome::Removed(web01())
    );
    assert_eq!(
        DeleteOutcome::from_remove(Ok(None)),
        DeleteOutcome::Absent,
        "Ok(None) is the nothing-was-deleted shape, not a success"
    );
    assert_eq!(
        DeleteOutcome::from_remove(Err("disk on fire".into())),
        DeleteOutcome::Failed("disk on fire".into())
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Slice 7 — the add step-sequence: typing, settling, validation
// ─────────────────────────────────────────────────────────────────────────────

/// The sequence is a text field, so the keystrokes it gets are text
/// keystrokes. They go into the field being answered and nowhere else:
/// the filter behind it is the user's, and a step that ate the filter
/// would leave the user with nothing to search with when the sequence
/// ended.
#[test]
fn typing_at_an_add_step_goes_to_the_field_not_the_filter() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let mut state = adding.state;
    for ch in "web-01".chars() {
        state = step(&state, plain(ch), Some(&web01())).state;
    }

    let sequence = match &state.phase {
        Phase::Add(sequence) => sequence,
        other => panic!("still expecting the add sequence, got {other:?}"),
    };

    assert_eq!(sequence.input, "web-01");
    assert_eq!(sequence.field, AddField::Alias, "typing does not advance");
    assert_eq!(
        state.query, "",
        "the filter behind the sequence is untouched"
    );
}

#[test]
fn backspace_at_an_add_step_edits_the_field_not_the_filter() {
    let adding = step(&ManageState::with_query("web"), ctrl('a'), Some(&web01()));
    let typed = step(&adding.state, plain('w'), Some(&web01()));
    let typed = step(&typed.state, plain('x'), Some(&web01()));

    let stepped = step(&typed.state, key(KeyCode::Backspace), Some(&web01()));

    let sequence = match &stepped.state.phase {
        Phase::Add(sequence) => sequence,
        other => panic!("still expecting the add sequence, got {other:?}"),
    };

    assert_eq!(sequence.input, "w", "the character comes off the field");
    assert_eq!(
        stepped.state.query, "web",
        "and not off the filter the user had typed before Ctrl+A"
    );
}

#[test]
fn the_chord_letters_still_type_at_an_add_step() {
    // `a`, `e` and `x` are filter text in the list and they are field
    // text here. Neither is a chord without the Ctrl.
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let mut state = adding.state;
    for ch in "ae".chars() {
        state = step(&state, plain(ch), Some(&web01())).state;
    }
    let last = step(&state, plain('x'), Some(&web01()));

    let sequence = match &last.state.phase {
        Phase::Add(sequence) => sequence,
        other => panic!("still expecting the add sequence, got {other:?}"),
    };
    assert_eq!(sequence.input, "aex");
    assert!(
        last.effects.is_empty(),
        "and none of them fired a management action: {:?}",
        last.effects
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Slice 8 — optionality: which fields may be left blank, and what happens
// when they are
//
// The rules come off the `Connection` model, not off taste:
//
//   alias, host  — `String`, and a Connection without one is not a
//                  Connection. Required.
//   port         — `u16`, and the SSH default is 22. Optional, with a
//                  default; but a value that is not a port is rejected
//                  rather than quietly coerced.
//   key, folder  — `Option<String>`. Optional, and *absent* when left
//                  empty — never `Some("")`, which would reach ssh as
//                  `-i ""`.
//
// `user` is not one of the spec's five steps, so the sequence cannot
// collect one; see `the_sequence_collects_the_five_fields_the_spec_names`.
// ─────────────────────────────────────────────────────────────────────────────

/// Type `text` into the live step, then press Enter.
fn answer(state: &ManageState, text: &str) -> manage::Step {
    let mut typed = state.clone();
    for ch in text.chars() {
        typed = step(&typed, plain(ch), Some(&web01())).state;
    }
    step(&typed, key(KeyCode::Enter), Some(&web01()))
}

/// The live sequence inside a state.
fn seq(state: &ManageState) -> &AddSequence {
    match &state.phase {
        Phase::Add(sequence) => sequence,
        other => panic!("expected the add sequence, got {other:?}"),
    }
}

/// A required field that was answered with nothing does not advance. The
/// user stays on the step, with what they had typed (nothing) still on
/// the line and a reason under it — they fix the field, they do not
/// start the step over.
#[test]
fn an_empty_alias_is_rejected_and_the_step_stays_on_alias() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let refused = step(&adding.state, key(KeyCode::Enter), Some(&web01()));

    let sequence = seq(&refused.state);
    assert_eq!(
        sequence.field,
        AddField::Alias,
        "a rejected answer must not move the user off the step they were on"
    );
    assert_eq!(
        sequence.error.as_deref(),
        Some("alias is required"),
        "and must tell them why: {sequence:?}"
    );
    assert!(
        refused.effects.is_empty(),
        "a rejection asks the driver for nothing: {:?}",
        refused.effects
    );
}

/// Whitespace is what an empty field looks like after a stray space, and
/// an alias of `"  "` is not an alias.
#[test]
fn a_whitespace_only_alias_is_rejected_as_empty() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let refused = answer(&adding.state, "   ");

    let sequence = seq(&refused.state);
    assert_eq!(sequence.field, AddField::Alias);
    assert_eq!(sequence.error.as_deref(), Some("alias is required"));
}

#[test]
fn an_empty_host_is_rejected_and_the_step_stays_on_host() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));
    let on_host = answer(&adding.state, "web-01");
    assert_eq!(seq(&on_host.state).field, AddField::Host, "setup");

    let refused = step(&on_host.state, key(KeyCode::Enter), Some(&web01()));

    let sequence = seq(&refused.state);
    assert_eq!(sequence.field, AddField::Host);
    assert_eq!(sequence.error.as_deref(), Some("host is required"));
    assert_eq!(
        sequence.draft.alias, "web-01",
        "the step already settled is not undone by the next one failing"
    );
}

/// The happy path between steps: an answer settles into the draft and the
/// sequence moves to the next field, starting it blank.
#[test]
fn enter_with_an_answer_settles_the_field_and_moves_to_the_next_step() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let advanced = answer(&adding.state, "  web-01  ");

    let sequence = seq(&advanced.state);
    assert_eq!(sequence.field, AddField::Host);
    assert_eq!(
        sequence.draft.alias, "web-01",
        "the settled value is trimmed — a leading space in an alias is a typo, not content"
    );
    assert_eq!(sequence.input, "", "the new step starts blank");
    assert_eq!(
        sequence.error, None,
        "and carries no complaint from the last one"
    );
    assert!(
        advanced.effects.is_empty(),
        "settling a field writes nothing: {:?}",
        advanced.effects
    );
}

/// Walking the whole sequence field by field, in the spec's order.
#[test]
fn the_sequence_walks_the_five_fields_the_spec_names() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let mut state = answer(&adding.state, "web-01").state;
    assert_eq!(seq(&state).field, AddField::Host);
    state = answer(&state, "10.0.0.4").state;
    assert_eq!(seq(&state).field, AddField::Port);
    state = answer(&state, "2222").state;
    assert_eq!(seq(&state).field, AddField::Key);
    state = answer(&state, "~/.ssh/id_ed25519").state;
    assert_eq!(seq(&state).field, AddField::Folder);

    assert_eq!(
        seq(&state).settled(),
        vec![
            ("Alias", Some("web-01".into())),
            ("Host", Some("10.0.0.4".into())),
            ("Port", Some("2222".into())),
            ("Key", Some("~/.ssh/id_ed25519".into())),
        ],
        "four steps settled behind the live one, in order"
    );
}

/// The state with Alias and Host answered, so the live step is Port.
fn at_port() -> ManageState {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));
    let on_host = answer(&adding.state, "web-01").state;
    answer(&on_host, "10.0.0.4").state
}

/// A port that is not a number is refused where it was typed. Silently
/// falling back to 22 here would build a Connection that connects to the
/// wrong machine, and the user would only find out from `ssh`.
#[test]
fn a_non_numeric_port_is_rejected_and_the_step_stays_on_port() {
    let refused = answer(&at_port(), "ssh");

    let sequence = seq(&refused.state);
    assert_eq!(sequence.field, AddField::Port);
    assert_eq!(
        sequence.error.as_deref(),
        Some("port must be a number from 1 to 65535")
    );
    assert_eq!(
        sequence.input, "ssh",
        "what the user typed stays on the line to be corrected"
    );
    assert!(refused.effects.is_empty());
}

#[test]
fn a_port_above_the_tcp_range_is_rejected() {
    let refused = answer(&at_port(), "70000");

    assert_eq!(seq(&refused.state).field, AddField::Port);
    assert_eq!(
        seq(&refused.state).error.as_deref(),
        Some("port must be a number from 1 to 65535")
    );
}

#[test]
fn port_zero_is_rejected() {
    let refused = answer(&at_port(), "0");

    assert_eq!(seq(&refused.state).field, AddField::Port);
    assert_eq!(
        seq(&refused.state).error.as_deref(),
        Some("port must be a number from 1 to 65535")
    );
}

#[test]
fn a_negative_port_is_rejected() {
    let refused = answer(&at_port(), "-1");

    assert_eq!(seq(&refused.state).field, AddField::Port);
}

/// The port is the one field that is optional *and* has a value when left
/// alone: the SSH default. Blank is not "absent" here — `Connection.port`
/// is a `u16`, there is no absent to settle to — so it settles as 22.
#[test]
fn an_empty_port_settles_as_the_ssh_default() {
    let advanced = step(&at_port(), key(KeyCode::Enter), Some(&web01()));

    let sequence = seq(&advanced.state);
    assert_eq!(
        sequence.field,
        AddField::Key,
        "blank is an answer here, so it advances"
    );
    assert_eq!(sequence.draft.port, "22");
    assert!(advanced.effects.is_empty());
}

#[test]
fn a_valid_port_settles_as_typed() {
    let advanced = answer(&at_port(), "2222");

    let sequence = seq(&advanced.state);
    assert_eq!(sequence.field, AddField::Key);
    assert_eq!(sequence.draft.port, "2222");
}

/// The state with Alias, Host and Port answered, so the live step is Key.
fn at_key() -> ManageState {
    answer(&at_port(), "2222").state
}

/// The state with everything but Folder answered.
fn at_folder() -> ManageState {
    answer(&at_key(), "~/.ssh/id_ed25519").state
}

/// An optional field accepts empty, and what it settles to is **absent**,
/// not the empty string. The difference is not pedantry: `Connection`'s
/// `key_path` is an `Option<String>`, and a `Some("")` reaches `ssh` as
/// `-i ""` — an identity file that is a zero-length path.
#[test]
fn an_empty_key_is_accepted_and_settles_as_absent() {
    let advanced = step(&at_key(), key(KeyCode::Enter), Some(&web01()));

    let sequence = seq(&advanced.state);
    assert_eq!(
        sequence.field,
        AddField::Folder,
        "blank is an answer for an optional field: it advances, it is not refused"
    );
    assert_eq!(
        sequence.settled()[3],
        ("Key", None),
        "and it settles as absent, not as an empty value"
    );
    assert_eq!(sequence.draft.key_path, "");
}

#[test]
fn an_empty_folder_is_accepted_and_settles_as_absent() {
    let finalised = step(&at_folder(), key(KeyCode::Enter), Some(&web01()));

    let draft = match &finalised.effects[..] {
        [Effect::Add { draft }] => draft,
        other => panic!("the last step must ask for exactly one add, got {other:?}"),
    };

    assert_eq!(draft.folder, "");
    assert_eq!(
        draft.key_path, "~/.ssh/id_ed25519",
        "the answered optional field is still there"
    );
}

/// A whitespace-only optional field is the same as a blank one: trimmed to
/// nothing, absent. A folder of `"  "` would render as `[  ]` in every
/// row of the list forever.
#[test]
fn a_whitespace_only_optional_field_settles_as_absent_too() {
    let advanced = answer(&at_key(), "   ");

    assert_eq!(seq(&advanced.state).field, AddField::Folder);
    assert_eq!(seq(&advanced.state).settled()[3], ("Key", None));
}

#[test]
fn an_answered_optional_field_settles_as_itself() {
    let advanced = answer(&at_key(), "~/.ssh/id_ed25519");

    assert_eq!(
        seq(&advanced.state).settled()[3],
        ("Key", Some("~/.ssh/id_ed25519".into()))
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Slice 9 — the note reports what the store actually did, and backing out
// writes nothing
// ─────────────────────────────────────────────────────────────────────────────

/// The last Enter completes the sequence and asks for the write. It does
/// not claim the write happened. The state machine owns no disk, and
/// `◇ added` is a claim about `connections.json`.
#[test]
fn finishing_the_sequence_asks_for_the_add_without_claiming_it_happened() {
    let finalised = step(&at_folder(), key(KeyCode::Enter), Some(&web01()));

    assert_eq!(
        finalised.state.phase,
        Phase::List,
        "the sequence is over; the frame is back on the list"
    );
    assert_eq!(
        finalised.state.trace, None,
        "the keystroke must not pre-commit an `added` note. The claim is \
         earned by the store's answer, folded in by `settle_add`"
    );
    assert_eq!(finalised.effects.len(), 1);
    assert!(
        matches!(&finalised.effects[0], Effect::Add { .. }),
        "and the one thing it asks for is the add: {:?}",
        finalised.effects
    );
}

/// The mirror case: a real write earns the note, and the note names the
/// Connection the store handed back — not a reconstruction of the draft.
#[test]
fn an_add_the_store_wrote_earns_the_added_note() {
    let finalised = step(&at_folder(), key(KeyCode::Enter), Some(&web01()));

    let settled = manage::settle_add(&finalised.state, &[web01()], Ok(web01()))
        .expect("a Connection that was really written settles onto the list");

    assert_eq!(
        settled.trace,
        Some(Trace::Added {
            connection: web01()
        })
    );
    assert_eq!(settled.phase, Phase::List);
}

/// A store that refused the write does not get to be reported as having
/// made one. The frame collapses and says so; `Trace::Added` is not
/// available to any path where the answer was `Err`.
#[test]
fn an_add_the_store_refused_does_not_claim_a_connection_was_added() {
    let finalised = step(&at_folder(), key(KeyCode::Enter), Some(&web01()));

    let outcome = manage::settle_add(
        &finalised.state,
        &[],
        Err("Permission denied (os error 13)".into()),
    );

    assert_eq!(
        outcome,
        Err("Permission denied (os error 13)".into()),
        "the store's own message must survive to be reported, not be swallowed"
    );
    assert!(
        !matches!(finalised.state.trace, Some(Trace::Added { .. })),
        "and nothing on the pre-settle state may read as added: {:?}",
        finalised.state.trace
    );
}

/// Esc walks the sequence back one step, with that step's answer put back
/// on the line so it can be corrected rather than retyped.
#[test]
fn esc_at_an_add_step_goes_back_to_the_previous_step_with_its_answer() {
    let back = step(&at_key(), key(KeyCode::Esc), Some(&web01()));

    let sequence = seq(&back.state);
    assert_eq!(sequence.field, AddField::Port, "one step back");
    assert_eq!(
        sequence.input, "2222",
        "with the answer that step settled on, on the line"
    );
    assert_eq!(sequence.error, None);
    assert!(
        back.effects.is_empty(),
        "walking back writes nothing: {:?}",
        back.effects
    );
}

/// Backing out of the first step is backing out of the sequence. Nothing
/// was ever written — no `Effect::Add` was ever emitted — and the note
/// says so, because the question the user has at that moment is whether
/// the half-filled form got saved.
#[test]
fn abandoning_the_sequence_mid_way_asks_for_nothing() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));
    let typed = step(&adding.state, plain('w'), Some(&web01()));

    let abandoned = step(&typed.state, key(KeyCode::Esc), Some(&web01()));

    assert_eq!(
        abandoned.state.phase,
        Phase::List,
        "backing off the first step leaves the sequence, not a step before it"
    );
    assert_eq!(
        abandoned.state.trace,
        Some(Trace::AddAbandoned),
        "and answers visibly that nothing was saved"
    );
    assert!(
        !abandoned
            .effects
            .iter()
            .any(|e| matches!(e, Effect::Add { .. })),
        "an abandoned sequence must never ask for a write: {:?}",
        abandoned.effects
    );
}

/// Going back and then forward again must not lose the answer that was
/// already settled on the field being re-entered.
///
/// `back()` puts the previous step's answer back on the line; the forward
/// advance owes the same rule. If it starts the re-entered field from an
/// empty line instead, a bare Enter re-settles that field from nothing —
/// silently wiping the answer the user gave the first time through. The
/// line must always show the truth about the field it names, whichever
/// direction the user arrived from.
#[test]
fn going_back_and_forward_again_re_seeds_the_line_from_the_stored_answer() {
    // Alias, Host, Port=2222 and Key=~/.ssh/id_ed25519 are answered;
    // Folder is live.
    let at_folder_state = at_folder();

    // Back twice: Folder → Key → Port. Port's line is correctly seeded
    // with "2222" by back().
    let at_key_again = step(&at_folder_state, key(KeyCode::Esc), Some(&web01())).state;
    let at_port_again = step(&at_key_again, key(KeyCode::Esc), Some(&web01())).state;
    assert_eq!(seq(&at_port_again).field, AddField::Port);
    assert_eq!(seq(&at_port_again).input, "2222");

    // Forward: settle Port (unchanged) and land on Key.
    let forward = step(&at_port_again, key(KeyCode::Enter), Some(&web01()));
    let sequence = seq(&forward.state);
    assert_eq!(sequence.field, AddField::Key);
    assert_eq!(
        sequence.input, "~/.ssh/id_ed25519",
        "the line must arrive holding the answer Key already settled to, \
         not an empty slot waiting to overwrite it"
    );

    // And a bare Enter on that re-seeded line keeps the key rather than
    // settling it absent.
    let settled_onward = step(&forward.state, key(KeyCode::Enter), Some(&web01()));
    let sequence = seq(&settled_onward.state);
    assert_eq!(sequence.field, AddField::Folder);
    assert_eq!(
        sequence.settled()[3],
        ("Key", Some("~/.ssh/id_ed25519".to_string())),
        "re-advancing must not destroy the settled Key"
    );
}

/// Walking the whole way back from the last step ends the same way, and
/// writes nothing at any point on the way.
#[test]
fn walking_all_the_way_back_writes_nothing() {
    let mut state = at_folder();

    for _ in 0..4 {
        state = step(&state, key(KeyCode::Esc), Some(&web01())).state;
        assert!(matches!(state.phase, Phase::Add(_)), "still walking back");
    }

    let last = step(&state, key(KeyCode::Esc), Some(&web01()));
    assert_eq!(last.state.phase, Phase::List);
    assert_eq!(last.state.trace, Some(Trace::AddAbandoned));
    assert!(last.effects.is_empty());
}

/// Ctrl+C in the middle of a sequence is the frame's cancel, not the
/// step's: it leaves, and it leaves nothing behind.
#[test]
fn ctrl_c_mid_sequence_cancels_the_frame_without_writing() {
    let cancelled = step(&at_folder(), ctrl('c'), Some(&web01()));

    assert_eq!(
        cancelled.effects,
        vec![Effect::Exit(InlineOutcome::Cancelled)]
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Slice 3 — nothing but `y` deletes
// ─────────────────────────────────────────────────────────────────────────────

/// The `(y/N)` prompt's default is No, so every answer that is not `y` has
/// to leave the Connection where it is. Asserted over the whole printable
/// range rather than a couple of examples: the acceptance criterion is "no
/// management action runs from a stray printable key", and a sweep is the
/// only way to hold that rather than gesture at it.
#[test]
fn no_answer_but_y_deletes_anything() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));

    for ch in (0x20u8..0x7f).map(|b| b as char) {
        let step = step(&armed.state, plain(ch), Some(&web01()));

        let deletes: Vec<&Effect> = step
            .effects
            .iter()
            .filter(|e| matches!(e, Effect::Delete { .. }))
            .collect();

        if ch == 'y' || ch == 'Y' {
            assert_eq!(
                deletes.len(),
                1,
                "{ch:?} is the confirming answer and must delete"
            );
        } else {
            assert!(
                deletes.is_empty(),
                "{ch:?} must not delete the Connection: {:?}",
                step.effects
            );
        }
    }
}

#[test]
fn n_declines_the_delete_and_leaves_the_connection_alone() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));

    let step = step(&armed.state, plain('N'), Some(&web01()));

    assert!(
        step.effects.is_empty(),
        "N is the prompt's default answer: it must ask for nothing, least of all a delete"
    );
    assert_eq!(step.state.phase, Phase::List, "N closes the confirm");
    assert_eq!(
        step.state.trace,
        Some(Trace::Declined {
            connection: web01()
        }),
        "the list must show that the delete was declined, not that it happened"
    );
}

#[test]
fn esc_declines_the_delete_rather_than_leaving_the_frame() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));

    let step = step(&armed.state, key(KeyCode::Esc), Some(&web01()));

    assert!(
        step.effects.is_empty(),
        "Esc at a delete confirm must not delete: {:?}",
        step.effects
    );
    assert_eq!(
        step.state.phase,
        Phase::List,
        "Esc backs out of the confirm step, not out of the frame — the user can \
         still leave with a second Esc, but one Esc must never be a delete"
    );
    assert_eq!(
        step.state.trace,
        Some(Trace::Declined {
            connection: web01()
        })
    );
}

/// The filter stays live *through* the confirm: a printable that is not an
/// answer declines the delete and goes into the search, so the user who hits
/// `Ctrl+X` and then keeps typing ends up filtering rather than deleting.
#[test]
fn a_stray_printable_at_the_confirm_declines_and_filters() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));

    let step = step(&armed.state, plain('w'), Some(&web01()));

    assert!(
        step.effects.is_empty(),
        "a stray printable must never delete: {:?}",
        step.effects
    );
    assert_eq!(
        step.state.query, "w",
        "the character the user typed is filter text, not an answer, so it must \
         reach the query rather than be swallowed by the confirm"
    );
    assert_eq!(step.state.phase, Phase::List);
}

#[test]
fn ctrl_x_arms_an_inline_confirm_for_the_selected_connection() {
    let state = ManageState::new();

    let step = step(&state, ctrl('x'), Some(&web01()));

    assert_eq!(
        step.state.phase,
        Phase::ConfirmDelete { target: web01() },
        "Ctrl+X must open the inline confirm naming the selected Connection"
    );
    assert!(
        step.effects.is_empty(),
        "nothing may be deleted before the user answers the confirm: {:?}",
        step.effects
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Ctrl+E — the in-place single-field editor (#37)
// ─────────────────────────────────────────────────────────────────────────────

/// `Ctrl+E` opens the editor on the Connection the chord was pressed over,
/// at its first field, with that field's current value already on the line.
///
/// The pre-fill is what makes this an *edit* rather than a second add: the
/// user sees the thing they are changing. Starting on Alias with an empty
/// line would ask them to retype an alias they already have.
#[test]
fn ctrl_e_opens_the_editor_on_the_selected_connection_at_its_first_field() {
    let step = step(&ManageState::new(), ctrl('e'), Some(&web01()));

    let Phase::Edit(editor) = &step.state.phase else {
        panic!("Ctrl+E must open the editor, got {:?}", step.state.phase);
    };
    assert_eq!(
        editor.target,
        web01(),
        "the editor must carry the Connection the chord was pressed over"
    );
    assert_eq!(editor.field, AddField::Alias);
    assert_eq!(
        editor.input, "web-01",
        "the line must arrive holding the field's current value"
    );
    assert!(
        step.effects.is_empty(),
        "opening an editor writes nothing: {:?}",
        step.effects
    );
}

#[test]
fn ctrl_e_with_nothing_selected_opens_nothing() {
    let step = step(&ManageState::new(), ctrl('e'), None);

    assert!(
        step.effects.is_empty(),
        "with no row under the cursor the frame must not invent a target to edit"
    );
    assert_eq!(step.state, ManageState::new());
}

/// The editor is scoped to the target captured at the chord, not to the
/// cursor. Arrowing across five fields must not be able to move the write
/// onto a neighbour — the same rule that stops a drifting cursor moving a
/// delete.
#[test]
fn the_edit_target_is_captured_at_the_chord_and_never_moves() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));

    // Walk the field selector all the way round and back.
    let mut state = armed.state.clone();
    for _ in 0..7 {
        state = step(&state, key(KeyCode::Right), Some(&web02())).state;
    }

    let Phase::Edit(editor) = &state.phase else {
        panic!("still editing, got {:?}", state.phase);
    };
    assert_eq!(
        editor.target,
        web01(),
        "the Connection being edited is the one under the chord, whatever the \
         cursor is over now"
    );
}

/// `Ctrl+E` clears the note the last action left.
///
/// A stale `◇ deleted` parked above "which field am I editing?" reads as
/// an answer to a question nobody has asked yet.
#[test]
fn ctrl_e_clears_the_note_the_last_action_left() {
    let with_note = ManageState {
        trace: Some(Trace::AddAbandoned),
        ..ManageState::new()
    };

    let step = step(&with_note, ctrl('e'), Some(&web01()));

    assert_eq!(step.state.trace, None);
}

#[test]
fn typing_in_the_editor_edits_the_field_not_the_filter_behind_it() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));
    let state = type_str(&armed.state, "db-01");

    let Phase::Edit(editor) = &state.phase else {
        panic!("still editing, got {:?}", state.phase);
    };
    assert_eq!(editor.input, "web-01db-01");
    assert_eq!(
        state.query, "",
        "the keystrokes belong to the field; the filter the user returns to is \
         untouched"
    );
}

/// The field selector re-seeds the line from the field it lands on.
///
/// This is what makes moving between fields safe: the line always shows the
/// truth about the field it names, so a user who arrows from Host to Port
/// and hits Enter cannot write a hostname into the port.
#[test]
fn moving_the_field_replaces_the_line_with_that_fields_actual_value() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));

    let on_host = step(&armed.state, key(KeyCode::Right), Some(&web01())).state;

    let Phase::Edit(editor) = &on_host.phase else {
        panic!("still editing, got {:?}", on_host.phase);
    };
    assert_eq!(editor.field, AddField::Host);
    assert_eq!(
        editor.input, "10.0.0.4",
        "the line must show the Host, not the Alias it was showing a keystroke ago"
    );
}

#[test]
fn the_field_selector_walks_the_five_fields_in_the_specs_order() {
    let mut state = step(&ManageState::new(), ctrl('e'), Some(&web01())).state;
    let mut seen = vec![field_of(&state)];

    for _ in 0..4 {
        state = step(&state, key(KeyCode::Right), Some(&web01())).state;
        seen.push(field_of(&state));
    }

    assert_eq!(
        seen,
        vec![
            AddField::Alias,
            AddField::Host,
            AddField::Port,
            AddField::Key,
            AddField::Folder
        ],
        "the editor cycles the same five fields, in the same order, the add \
         sequence walks"
    );
}

/// The selector wraps at both ends.
///
/// A `←` that silently dies at the first field reads as a broken keybinding
/// on a rail that has already promised the arrows move the field.
#[test]
fn the_field_selector_wraps_at_both_ends() {
    let at_alias = step(&ManageState::new(), ctrl('e'), Some(&web01())).state;

    let back = step(&at_alias, key(KeyCode::Left), Some(&web01())).state;
    assert_eq!(
        field_of(&back),
        AddField::Folder,
        "← off the first field must land on the last, not nowhere"
    );

    let at_folder = step(&at_alias, key(KeyCode::Left), Some(&web01())).state;
    assert_eq!(
        field_of(&at_folder),
        AddField::Folder,
        "one step back from the first field is the last field"
    );

    let forward = step(&at_folder, key(KeyCode::Right), Some(&web01())).state;
    assert_eq!(
        field_of(&forward),
        AddField::Alias,
        "→ off the last field must come back round to the first"
    );
}

#[test]
fn enter_on_an_unchanged_field_still_asks_for_the_write_it_was_answered() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));

    let step = step(&armed.state, key(KeyCode::Enter), Some(&web01()));

    let Effect::Update { draft, .. } = &step.effects[0] else {
        panic!("Enter must ask for the write, got {:?}", step.effects);
    };
    assert_eq!(
        draft.alias, "web-01",
        "the unchanged value is written back as it was read"
    );
}

/// Enter writes exactly one field and leaves every other field of the
/// Connection exactly as it was read.
///
/// The editor changes one thing; the draft it emits must not be a blank
/// form with one answer in it. A draft built from `Default` rather than
/// from the target would silently wipe the host, the user and the folder
/// off a Connection whose alias was being corrected.
#[test]
fn committing_a_field_changes_only_that_field() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));
    let state = type_str(&armed.state, "x");

    let step = step(&state, key(KeyCode::Enter), Some(&web01()));

    let Effect::Update { target, draft } = &step.effects[0] else {
        panic!("Enter must ask for the write, got {:?}", step.effects);
    };
    assert_eq!(target, &web01(), "the request names the target");
    assert_eq!(draft.alias, "web-01x", "the edited field carries the edit");
    assert_eq!(draft.host, "10.0.0.4", "host untouched");
    assert_eq!(draft.user, "deploy", "user untouched");
    assert_eq!(draft.port, "22", "port untouched");
    assert_eq!(draft.folder, "prod", "folder untouched");
}

/// The id travels with the edit.
///
/// A write that minted a fresh id would orphan every reference to the
/// Connection the user was editing and leave the old row in the list.
#[test]
fn the_edit_carries_the_targets_id_not_a_new_one() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));
    let step = step(&armed.state, key(KeyCode::Enter), Some(&web01()));

    let Effect::Update { target, draft } = &step.effects[0] else {
        panic!("expected an Update, got {:?}", step.effects);
    };
    assert_eq!(target.id, "id-web-01");
    assert_eq!(
        draft.alias, "web-01",
        "the draft is the whole Connection, so the store can keep the id"
    );
}

/// The editor validates through the same `settle` the add sequence uses,
/// so a field cannot have two different rules depending on which chord
/// opened it.
#[test]
fn the_editor_applies_the_same_per_field_validation_as_the_add_sequence() {
    let on_port = focus(AddField::Port);
    assert_eq!(field_of(&on_port), AddField::Port);

    // Out of range: refused, stays on the field, writes nothing.
    let cleared = backspace_n(&on_port, "22".chars().count());
    let bad = type_str(&cleared, "99999");
    let refused = step(&bad, key(KeyCode::Enter), Some(&web01()));

    let Phase::Edit(editor) = &refused.state.phase else {
        panic!(
            "a refused port must stay on the field, got {:?}",
            refused.state.phase
        );
    };
    assert_eq!(
        editor.error.as_deref(),
        Some("port must be a number from 1 to 65535"),
        "the same sentence the add sequence gives"
    );
    assert!(
        refused.effects.is_empty(),
        "a refused field must not write: {:?}",
        refused.effects
    );

    // Empty: the SSH default, the same answer the add sequence gives.
    let cleared = backspace_n(&bad, "99999".chars().count());
    let settled = step(&cleared, key(KeyCode::Enter), Some(&web01()));
    let Effect::Update { draft, .. } = &settled.effects[0] else {
        panic!(
            "an empty port settles to the default, got {:?}",
            settled.effects
        );
    };
    assert_eq!(draft.port, "22");
}

#[test]
fn a_refused_edit_keeps_the_typed_text_and_the_target() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));
    let state = type_str(&armed.state, "");

    // Blank a required field and press Enter.
    let cleared = backspace_n(&state, "web-01".len());
    let refused = step(&cleared, key(KeyCode::Enter), Some(&web01()));

    let Phase::Edit(editor) = &refused.state.phase else {
        panic!("must still be editing, got {:?}", refused.state.phase);
    };
    assert_eq!(editor.target, web01(), "the target survives the refusal");
    assert_eq!(editor.input, "", "what the user typed stays on the line");
    assert_eq!(
        editor.error.as_deref(),
        Some("alias is required"),
        "the refusal names the field"
    );
    assert!(refused.effects.is_empty());
}

#[test]
fn esc_abandons_the_edit_and_promises_nothing_was_saved() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));
    let typed = type_str(&armed.state, "-typo");

    let step = step(&typed, key(KeyCode::Esc), Some(&web01()));

    assert_eq!(step.state.phase, Phase::List);
    assert_eq!(step.state.trace, Some(Trace::EditAbandoned));
    assert!(
        step.effects.is_empty(),
        "abandoning an edit must not write: {:?}",
        step.effects
    );
}

#[test]
fn ctrl_c_inside_the_editor_cancels_the_frame_without_writing() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));

    let step = step(&armed.state, ctrl('c'), Some(&web01()));

    assert_eq!(step.effects, vec![Effect::Exit(InlineOutcome::Cancelled)]);
}

/// List movement does nothing while the editor is open.
///
/// The target is captured at the chord, so there is nothing for the cursor
/// to change — and a stray arrow that moved the target mid-edit would put
/// the write somewhere the user never pointed.
#[test]
fn list_movement_does_nothing_while_the_editor_is_open() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));

    for code in [KeyCode::Up, KeyCode::Down] {
        let step = step(&armed.state, key(code), Some(&web02()));
        assert_eq!(
            step.state, armed.state,
            "{code:?} must not change the edit in any way"
        );
        assert!(step.effects.is_empty());
    }
}

/// Clearing an optional field on the edit path makes it *absent*, not an
/// empty string.
///
/// A `key_path: Some("")` would reach ssh as `-i ""`. The same rule the
/// add sequence enforces, reached through the same `settle`.
#[test]
fn clearing_an_optional_field_makes_it_absent() {
    let with_key = Connection {
        id: "id-web-01".into(),
        alias: "web-01".into(),
        host: "10.0.0.4".into(),
        user: "deploy".into(),
        port: 22,
        key_path: Some("~/.ssh/id_ed25519".into()),
        folder: Some("prod".into()),
    };

    let armed = step(&ManageState::new(), ctrl('e'), Some(&with_key));
    // Alias → Host → Port → Key
    let on_key = step(
        &step(
            &step(&armed.state, key(KeyCode::Right), Some(&with_key)).state,
            key(KeyCode::Right),
            Some(&with_key),
        )
        .state,
        key(KeyCode::Right),
        Some(&with_key),
    )
    .state;
    assert_eq!(field_of(&on_key), AddField::Key);

    let cleared = backspace_n(&on_key, "~/.ssh/id_ed25519".chars().count());
    let step = step(&cleared, key(KeyCode::Enter), Some(&with_key));

    let Effect::Update { draft, .. } = &step.effects[0] else {
        panic!("expected an Update, got {:?}", step.effects);
    };
    assert_eq!(
        draft.key_path, "",
        "the draft carries the empty answer; the store turns it into absent"
    );
}

/// The refreshed list must actually *show* the Connection the note names.
///
/// A filter typed before `Ctrl+A` can hide the new row: add `db-01` while
/// the filter reads `web` and the list would answer `No matches` under a
/// note claiming `db-01` was added. The write clears the filter and puts
/// the cursor on the row it made, so the note and the rows agree.
#[test]
fn a_settled_add_clears_a_filter_that_would_hide_the_new_connection() {
    let filtered = ManageState::with_query("web");
    let db01 = Connection {
        id: "id-db-01".into(),
        alias: "db-01".into(),
        ..web01()
    };

    let settled =
        manage::settle_add(&filtered, &[web01(), db01.clone()], Ok(db01.clone())).unwrap();

    assert_eq!(
        settled.query, "",
        "the filter that would hide the new row is cleared by the write"
    );
    assert_eq!(
        settled.selection, 1,
        "and the cursor lands on the new Connection, the second row"
    );
    assert_eq!(settled.trace, Some(Trace::Added { connection: db01 }));
}

/// The same promise on the edit path: renaming a Connection out from under
/// the filter must not leave the changed row invisible.
#[test]
fn a_settled_edit_clears_a_filter_that_would_hide_the_changed_connection() {
    let filtered = ManageState::with_query("web");
    let renamed = Connection {
        alias: "db-01".into(),
        ..web01()
    };

    let settled = manage::settle_edit(
        &filtered,
        std::slice::from_ref(&renamed),
        &web01(),
        EditOutcome::Updated(renamed.clone()),
    )
    .unwrap();

    assert_eq!(
        settled.query, "",
        "the write clears the filter the changed row no longer matches"
    );
    assert_eq!(settled.selection, 0, "and the cursor is on the changed row");
    assert_eq!(
        settled.trace,
        Some(Trace::Edited {
            connection: renamed
        })
    );
}

/// A failed edit leaves the user's filter alone — nothing was written, so
/// there is no new row to reveal and no reason to disturb the list.
#[test]
fn a_failed_edit_keeps_the_filter_the_user_had() {
    let filtered = ManageState::with_query("web");

    let absent = manage::settle_edit(&filtered, &[], &web01(), EditOutcome::Absent).unwrap();

    assert_eq!(absent.query, "web", "nothing was written; the filter stays");
}

/// `settle_edit` is the only place the frame may claim an edit happened.
#[test]
fn settle_edit_grants_the_note_only_on_a_real_write() {
    let state = ManageState::new();
    let edited = Connection {
        alias: "web-01x".into(),
        ..web01()
    };

    let ok = manage::settle_edit(
        &state,
        std::slice::from_ref(&edited),
        &web01(),
        EditOutcome::Updated(edited.clone()),
    )
    .unwrap();
    assert_eq!(
        ok.trace,
        Some(Trace::Edited { connection: edited }),
        "a write that happened earns `edited`"
    );
    assert_eq!(ok.phase, Phase::List, "and returns to the list");

    let absent = manage::settle_edit(&state, &[], &web01(), EditOutcome::Absent).unwrap();
    assert_eq!(
        absent.trace,
        Some(Trace::EditFailed {
            connection: web01()
        }),
        "a `y` that changed nothing must not be reported as `edited`"
    );

    let failed = manage::settle_edit(
        &state,
        &[],
        &web01(),
        EditOutcome::Failed("disk on fire".into()),
    );
    assert_eq!(failed, Err("disk on fire".to_string()));
}

#[test]
fn the_edit_outcome_classifies_the_store_answer() {
    assert_eq!(
        EditOutcome::from_update(Ok(Some(web01()))),
        EditOutcome::Updated(web01())
    );
    assert_eq!(EditOutcome::from_update(Ok(None)), EditOutcome::Absent);
    assert_eq!(
        EditOutcome::from_update(Err("nope".into())),
        EditOutcome::Failed("nope".into())
    );
}

/// The editor is not the add sequence.
///
/// Story 26 asks for a small correction to take *one* step. If `Ctrl+E`
/// opened the five-field walk, correcting a port would be four steps of
/// nothing followed by one step of the thing the user wanted.
#[test]
fn the_edit_is_one_step_not_a_five_field_walk() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));

    let step = step(&armed.state, key(KeyCode::Enter), Some(&web01()));

    assert_eq!(
        step.effects.len(),
        1,
        "one Enter on the opened editor must be the whole edit"
    );
    assert!(matches!(step.effects[0], Effect::Update { .. }));
    assert!(
        !matches!(step.state.phase, Phase::Add(_)),
        "Ctrl+E must never open the add sequence"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Open the editor and walk the selector to `field`.
fn focus(field: AddField) -> ManageState {
    let mut state = step(&ManageState::new(), ctrl('e'), Some(&web01())).state;
    while field_of(&state) != field {
        state = step(&state, key(KeyCode::Right), Some(&web01())).state;
    }
    state
}

fn field_of(state: &ManageState) -> AddField {
    match &state.phase {
        Phase::Edit(editor) => editor.field,
        other => panic!("expected the editor, got {other:?}"),
    }
}

fn type_str(state: &ManageState, text: &str) -> ManageState {
    let mut s = state.clone();
    for ch in text.chars() {
        s = step(&s, key(KeyCode::Char(ch)), Some(&web01())).state;
    }
    s
}

fn backspace_n(state: &ManageState, n: usize) -> ManageState {
    let mut s = state.clone();
    for _ in 0..n {
        s = step(&s, key(KeyCode::Backspace), Some(&web01())).state;
    }
    s
}
