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
use sshm::manage::{self, step, DeleteOutcome, Effect, ManageState, Phase, Trace};

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
                Effect::Delete { .. } | Effect::BeginAdd | Effect::Exit(_)
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

/// `Ctrl+E` is routed, not swallowed: it hands the selected Connection to
/// the edit path, which is exactly what Enter means under `sshm manage`
/// (#35's cut-over contract). #37 replaces the destination with the
/// in-place single-field editor; the route is already here.
#[test]
fn ctrl_e_routes_the_selected_connection_to_the_edit_path() {
    let step = step(&ManageState::new(), ctrl('e'), Some(&web01()));

    assert_eq!(
        step.effects,
        vec![Effect::Exit(InlineOutcome::Picked(web01()))],
        "Ctrl+E must leave the frame with the selected Connection on the edit route"
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

/// `Ctrl+A` is routed to the add path and leaves a dim note. The interactive
/// add sequence itself is #37's; what lands here is the chord being read,
/// acted on, and answered visibly — not a dead key.
#[test]
fn ctrl_a_routes_to_the_add_path_and_leaves_a_note() {
    let step = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    assert_eq!(
        step.effects,
        vec![Effect::BeginAdd],
        "Ctrl+A must reach the driver as an add request"
    );
    assert_eq!(
        step.state.trace,
        Some(Trace::AddRequested),
        "and the frame must answer visibly rather than swallow the chord"
    );
    assert_eq!(
        step.state.phase,
        Phase::List,
        "the frame stays on the list: the user keeps their filter and their cursor"
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
        next.state.trace,
        Some(Trace::AddRequested),
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
