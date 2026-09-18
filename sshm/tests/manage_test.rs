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
//!
//! The two form flows — `Ctrl+A` and `Ctrl+E` — are now **one form map**: six
//! field rows over a `▶` submit row, walked by a single [`FormCursor`] across
//! a draft that is live rather than staged one step at a time. That reshapes
//! almost every assertion below, and it inverts one rule in particular. The
//! stepped add used to *trap* the user on a field that would not validate;
//! the map traps nobody. Movement is free, the row's own glyph says what is
//! wrong, and the only gate in the whole form is the `▶` row.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sshm::config::{Connection, ImportReport};
use sshm::connections::ConnectionDraft;
use sshm::inline::InlineOutcome;
use sshm::manage::{
    self, step, AddField, AddSequence, DeleteOutcome, EditOutcome, Effect, EditSequence, FormCursor,
    ImportOutcome, ManageState, MapRow, Phase, RowGlyph, Trace,
};

/// The port's refusal sentence, spelled once. The dash in `1–65535` is an
/// en dash, not a hyphen, and retyping it wrong in five places is exactly
/// the kind of drift a constant exists to prevent.
const PORT_REFUSAL: &str = "port must be 1\u{2013}65535 (e.g. 22)";

/// What `refusal_line` puts between two problems.
const JOIN: &str = " \u{b7} ";

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

/// `Ctrl+A` opens the form map — `Alias` `Host` `User` `Port` `Key`
/// `Folder` over the `▶ Add connection` row — with the cursor on the
/// first field and every row blank.
///
/// Nothing is asked of the driver yet: opening a question is not an
/// effect. The map is pure state until the `▶` row is accepted.
///
/// Six rows, not the five the original ticket walked. `User` is the sixth,
/// and it is here because its absence was a bug rather than a design; see
/// `user_is_settable_during_add_and_lands_in_the_draft`.
#[test]
fn ctrl_a_opens_the_add_map_at_the_alias_row() {
    let step = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let sequence = match &step.state.phase {
        Phase::Add(sequence) => sequence,
        other => panic!("Ctrl+A must open the add map, got {other:?}"),
    };

    assert_eq!(
        sequence.cursor,
        FormCursor::Field(AddField::Alias),
        "the map opens on Alias, the first of the spec's six fields"
    );
    assert_eq!(
        sequence.draft,
        ConnectionDraft::default(),
        "and opens with nothing typed into any row: {:?}",
        sequence.draft
    );
    assert_eq!(
        sequence.rows().len(),
        AddField::ORDER.len(),
        "the map draws one row per field, with the submit row below them"
    );
    assert!(
        step.effects.is_empty(),
        "opening a row must ask the driver for nothing: {:?}",
        step.effects
    );
}

/// The add map is reachable from the list whatever the list is doing,
/// and takes the frame off the list without leaving the frame.
#[test]
fn ctrl_a_opens_the_sequence_from_a_live_filter_without_losing_it() {
    let filtered = ManageState::with_query("web");

    let step = step(&filtered, ctrl('a'), Some(&web01()));

    assert!(matches!(step.state.phase, Phase::Add(_)));
    assert_eq!(
        step.state.query, "web",
        "the filter is the user's; the map borrows the frame, not the query"
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
// The map's cursor: travel, and the fact that nothing on a row can stop it
//
// The stepped add used to refuse movement out of a field that would not
// validate, which is what made `Ctrl+A` feel like an interrogation. The map
// inverts that: **movement is never refused**, from any row, whatever the
// row holds. `↑`/`↓` saturate at the ends because they are the reading
// keys, `Tab`/`BackTab` wrap because they are the travel keys, and `Enter`
// on a field is just another way of moving down one. The single gate in the
// whole form is the `▶` row, and nothing else.
// ─────────────────────────────────────────────────────────────────────────────

/// The map is six field rows under a seven-row cursor. Pinned as numbers
/// because the frame's constant height is counted from them: 6 fields +
/// 1 rule row + 1 `▶` row = the 8 rows `VISIBLE_ROWS` promises.
#[test]
fn the_map_is_six_field_rows_under_a_seven_row_cursor() {
    assert_eq!(
        AddField::ORDER,
        [
            AddField::Alias,
            AddField::Host,
            AddField::User,
            AddField::Port,
            AddField::Key,
            AddField::Folder
        ],
        "six fields, in the order the map draws them"
    );
    assert_eq!(
        AddField::ORDER
            .iter()
            .map(|f| f.label())
            .collect::<Vec<_>>(),
        vec!["Alias", "Host", "User", "Port", "Key", "Folder"],
        "each with its own label"
    );
    assert_eq!(
        FormCursor::ROWS,
        7,
        "the six fields plus the submit row the cursor can stand on"
    );
    assert_eq!(FormCursor::Field(AddField::Alias).index(), 0);
    assert_eq!(FormCursor::Field(AddField::Folder).index(), 5);
    assert_eq!(FormCursor::Submit.index(), 6);
    assert!(FormCursor::Submit.is_submit());
    assert!(!FormCursor::Field(AddField::Folder).is_submit());
}

/// `Tab` and `BackTab` wrap at both ends.
///
/// A `Tab` that dies at the bottom of a seven-row map is a Tab the user
/// has to count, so the travel keys come back round instead.
#[test]
fn tab_and_backtab_wrap_the_map() {
    let adding = add_at(FormCursor::Field(AddField::Alias));

    let wrapped = step(&adding, key(KeyCode::BackTab), Some(&web01()));
    assert_eq!(
        cursor_of(&wrapped.state),
        FormCursor::Submit,
        "BackTab off the first row lands on the `▶` row, not nowhere"
    );

    let wrapped_back = step(&wrapped.state, key(KeyCode::Tab), Some(&web01()));
    assert_eq!(
        cursor_of(&wrapped_back.state),
        FormCursor::Field(AddField::Alias),
        "and Tab off the `▶` row comes back round to the top"
    );

    assert!(
        wrapped.effects.is_empty() && wrapped_back.effects.is_empty(),
        "travelling the map asks the driver for nothing: {:?} {:?}",
        wrapped.effects,
        wrapped_back.effects
    );
}

/// `Up`/`Down` saturate rather than wrap.
///
/// These are the reading keys: a user walking the map expects to stop at
/// the end, not to be teleported to the top and have to re-read the whole
/// thing to find where they were.
#[test]
fn up_and_down_saturate_at_the_ends_of_the_map() {
    let at_top = add_at(FormCursor::Field(AddField::Alias));

    for _ in 0..3 {
        let stuck = step(&at_top, key(KeyCode::Up), Some(&web01()));
        assert_eq!(
            cursor_of(&stuck.state),
            FormCursor::Field(AddField::Alias),
            "`↑` at the top stays at the top"
        );
    }

    let at_bottom = add_at(FormCursor::Submit);
    for _ in 0..3 {
        let stuck = step(&at_bottom, key(KeyCode::Down), Some(&web01()));
        assert_eq!(
            cursor_of(&stuck.state),
            FormCursor::Submit,
            "`↓` at the `▶` row stays on the `▶` row"
        );
    }
}

/// Enter on a field is a movement key, not a validation gate: it advances
/// whatever the field holds, including nothing at all.
///
/// This is the inversion of the old stepped add, which held the user on
/// `◆ Alias` until they typed something. Here the empty Alias simply
/// advances, its `○` already saying it is still needed, and the refusal —
/// if there is going to be one — waits at the `▶` row.
#[test]
fn enter_on_a_field_never_refuses_even_when_that_field_is_invalid() {
    let mut state = add_at(FormCursor::Field(AddField::Alias));

    // Six Enters over a completely blank form — every required row is
    // invalid the whole way — and not one of them refuses to move.
    let expected = [
        FormCursor::Field(AddField::Host),
        FormCursor::Field(AddField::User),
        FormCursor::Field(AddField::Port),
        FormCursor::Field(AddField::Key),
        FormCursor::Field(AddField::Folder),
        FormCursor::Submit,
    ];
    for (i, want) in expected.iter().enumerate() {
        let moved = step(&state, key(KeyCode::Enter), Some(&web01()));
        assert_eq!(
            cursor_of(&moved.state),
            *want,
            "Enter #{i} must advance off a blank required field, not trap the \
             user on it"
        );
        assert_eq!(
            error_of(&moved.state),
            None,
            "and must not complain on the way past it"
        );
        assert!(
            moved.effects.is_empty(),
            "moving never writes: {:?}",
            moved.effects
        );
        state = moved.state;
    }
}

/// Enter on a field that is actively invalid still advances, and the bad
/// value is kept rather than discarded.
#[test]
fn enter_advances_off_a_field_holding_an_invalid_value() {
    let typed = type_str(&add_at(FormCursor::Field(AddField::Port)), "ssh");

    let moved = step(&typed, key(KeyCode::Enter), Some(&web01()));

    assert_eq!(
        cursor_of(&moved.state),
        FormCursor::Field(AddField::Key),
        "a port of `ssh` must not pin the cursor to the Port row"
    );
    assert_eq!(
        add_seq(&moved.state).draft.port,
        "ssh",
        "what the user typed is kept in the draft to be corrected later"
    );
    assert!(moved.effects.is_empty());
}

/// `Esc` at index 0 abandons the whole form; `Esc` anywhere else is one
/// row up.
///
/// Backing off the top row is backing out of the thing entirely, and the
/// note answers the only question the user has at that moment: did the
/// half-filled form get saved? It did not.
#[test]
fn esc_at_the_top_abandons_and_esc_elsewhere_moves_up_one_row() {
    let at_key = add_at(FormCursor::Field(AddField::Key));

    let up = step(&at_key, key(KeyCode::Esc), Some(&web01()));
    assert_eq!(
        cursor_of(&up.state),
        FormCursor::Field(AddField::Port),
        "`Esc` mid-map is one row up, not an exit"
    );
    assert!(matches!(up.state.phase, Phase::Add(_)));
    assert!(up.effects.is_empty());

    let abandoned = step(
        &add_at(FormCursor::Field(AddField::Alias)),
        key(KeyCode::Esc),
        None,
    );
    assert_eq!(
        abandoned.state.phase,
        Phase::List,
        "`Esc` on the first row leaves the form"
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
        "an abandoned form must never ask for a write: {:?}",
        abandoned.effects
    );
}

/// Typing and Backspace are no-ops while the cursor is on the `▶` row.
///
/// The alternative is worse in a way that is easy to miss: appending to
/// whichever field was focused *last* means the user is changing a row they
/// are not looking at, several rows above the one under the cursor. Swallow
/// the keystroke instead.
#[test]
fn typing_and_backspace_are_no_ops_on_the_submit_row() {
    let at_submit = goto(&filled_add(), FormCursor::Submit);
    let before = add_seq(&at_submit).draft.clone();

    let typed = step(&at_submit, plain('z'), Some(&web01()));
    assert_eq!(
        add_seq(&typed.state).draft, before,
        "a character typed on the `▶` row must not be appended to any field"
    );

    let deleted = step(&at_submit, key(KeyCode::Backspace), Some(&web01()));
    assert_eq!(
        add_seq(&deleted.state).draft, before,
        "and Backspace on the `▶` row must not eat a character off the last field"
    );

    assert!(typed.effects.is_empty() && deleted.effects.is_empty());
    assert_eq!(
        cursor_of(&typed.state),
        FormCursor::Submit,
        "the no-op must not move the cursor either"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Slice 7 — the map is a text field: typing goes to the row under the cursor
// ─────────────────────────────────────────────────────────────────────────────

/// The row under the cursor is a text field, so the keystrokes the map gets
/// are text keystrokes. They go into that row and nowhere else: the filter
/// behind it is the user's, and a form that ate the filter would leave the
/// user with nothing to search with when it ended.
#[test]
fn typing_at_an_add_step_goes_to_the_field_not_the_filter() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let mut state = adding.state;
    for ch in "web-01".chars() {
        state = step(&state, plain(ch), Some(&web01())).state;
    }

    let sequence = add_seq(&state);

    assert_eq!(sequence.draft.alias, "web-01");
    assert_eq!(
        sequence.cursor,
        FormCursor::Field(AddField::Alias),
        "typing does not advance"
    );
    assert_eq!(
        state.query, "",
        "the filter behind the map is untouched"
    );
}

#[test]
fn backspace_at_an_add_step_edits_the_field_not_the_filter() {
    let adding = step(&ManageState::with_query("web"), ctrl('a'), Some(&web01()));
    let typed = step(&adding.state, plain('w'), Some(&web01()));
    let typed = step(&typed.state, plain('x'), Some(&web01()));

    let stepped = step(&typed.state, key(KeyCode::Backspace), Some(&web01()));

    let sequence = add_seq(&stepped.state);

    assert_eq!(sequence.draft.alias, "w", "the character comes off the field");
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

    let sequence = add_seq(&last.state);
    assert_eq!(sequence.draft.alias, "aex");
    assert!(
        last.effects.is_empty(),
        "and none of them fired a management action: {:?}",
        last.effects
    );
}

/// **The bug this test exists to kill.**
///
/// The original five-step add had no `User` step, so every Connection added
/// through it carried an empty `user` and rendered as
/// `(@10.0.0.4:22)` — a Connection whose login nobody could name, fixable
/// only afterwards. `sshm add --user` had always accepted one; the
/// interactive flow simply had nowhere to put it. That was a spec gap, not
/// a design decision, and the sixth row of the map closes it.
///
/// Before the map there was no User row to type into, so this test could not
/// have existed — and every Connection the old flow produced was wrong in
/// exactly the way it catches.
#[test]
fn user_is_settable_during_add_and_lands_in_the_draft() {
    let typed =
        type_str(&goto(&add_with_required_filled(), FormCursor::Field(AddField::User)), "deploy");

    assert_eq!(
        add_seq(&typed).draft.user,
        "deploy",
        "the User row must be typeable and must hold what was typed"
    );
    assert_eq!(
        add_seq(&typed).cursor,
        FormCursor::Field(AddField::User),
        "and typing there must not move the cursor off it"
    );
    assert_eq!(
        glyph_of(&typed, AddField::User),
        RowGlyph::Valid,
        "the row must read as filled"
    );

    // And it survives the walk to the `▶` row: the value lives in the
    // draft, not on a line that is discarded when the cursor moves on.
    let added = submit(&typed);
    let draft = only_add(&added);
    assert_eq!(
        draft.user, "deploy",
        "the user the operator typed must reach the draft the store is handed"
    );
}

/// The User row is optional: leaving it blank is a real answer, not a
/// missing one, and it never blocks the submit. `Connection::user` empty
/// means "no `user@`, let ssh use the local login name".
#[test]
fn a_blank_user_is_accepted_and_never_blocks_the_submit() {
    let at_user = goto(&filled_add(), FormCursor::Field(AddField::User));
    let cleared = backspace_n(&at_user, "deploy".chars().count());

    assert_eq!(add_seq(&cleared).draft.user, "", "setup");
    assert!(
        add_seq(&cleared).ready(),
        "a blank User must not make the form unready: {:?}",
        add_seq(&cleared).problems()
    );

    let added = submit(&cleared);
    assert!(
        matches!(&added.effects[..], [Effect::Add { .. }]),
        "the ▶ row must accept a form whose User is blank: {:?}",
        added.effects
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Slice 8 — optionality: which rows may be left blank, and what happens
// when they are
//
// The rules come off the `Connection` model, not off taste:
//
//   alias, host  — `String`, and a Connection without one is not a
//                  Connection. Required.
//   user         — a plain `String` that ssh reads as "no `user@`".
//                  Optional; blank is the local login name.
//   port         — `u16`, and the SSH default is 22. Optional, with a
//                  default; but a value that is not a port is rejected
//                  rather than quietly coerced.
//   key, folder  — `Option<String>`. Optional, and *absent* when left
//                  empty — never `Some("")`, which would reach ssh as
//                  `-i ""`.
//
// What changed with the map is *where* these bite. They no longer stop the
// user leaving a row; they stop the `▶` row being accepted.
// ─────────────────────────────────────────────────────────────────────────────

/// A required row left empty is not refused *at the row* — Enter walks
/// straight past it — but it is a problem the map reports, and the `▶` row
/// will not accept it.
#[test]
fn an_empty_alias_is_a_problem_the_map_reports_but_never_a_wall() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let advanced = step(&adding.state, key(KeyCode::Enter), Some(&web01()));

    let sequence = add_seq(&advanced.state);
    assert_eq!(
        sequence.cursor,
        FormCursor::Field(AddField::Host),
        "an empty Alias does not hold the cursor"
    );
    assert_eq!(
        glyph_of(&advanced.state, AddField::Alias),
        RowGlyph::Needed,
        "but the row says it is needed: {:?}",
        sequence.rows()
    );
    assert!(
        !sequence.ready(),
        "and the form is not submittable while it is empty"
    );
}

/// Whitespace is what an empty field looks like after a stray space, and
/// an alias of `"  "` is not an alias.
#[test]
fn a_whitespace_only_alias_is_rejected_as_empty() {
    // Host is filled so the Alias is the only thing wrong with the form,
    // and the reported problem isolates the whitespace rule.
    let with_host = type_str(&add_at(FormCursor::Field(AddField::Host)), "10.0.0.4");
    let typed = type_str(&goto(&with_host, FormCursor::Field(AddField::Alias)), "   ");

    assert_eq!(
        glyph_of(&typed, AddField::Alias),
        RowGlyph::Needed,
        "three spaces are still an empty required field"
    );
    assert_eq!(
        problems_of(&typed),
        vec![("Alias", "alias is required".to_string())],
        "and the problem is reported as the required-field one, not as content"
    );
    assert!(
        !add_seq(&typed).ready(),
        "and the form still will not submit: {:?}",
        add_seq(&typed).problems()
    );
}

/// An empty Host is a problem, and it does not undo the Alias that was
/// already filled in above it.
#[test]
fn an_empty_host_is_a_problem_that_leaves_the_alias_alone() {
    let typed = type_str(&add_at(FormCursor::Field(AddField::Alias)), "web-01");
    let on_host = step(&typed, key(KeyCode::Enter), Some(&web01()));

    assert_eq!(
        glyph_of(&on_host.state, AddField::Host),
        RowGlyph::Needed,
        "the Host row says it is needed"
    );
    assert_eq!(
        add_seq(&on_host.state).draft.alias, "web-01",
        "the row already filled is not undone by the next one being empty"
    );
}

/// The happy path between rows: what is typed lands in the draft, the
/// cursor moves on, and nothing is written.
///
/// Note what is *not* true here: the live draft holds `  web-01  ` with the
/// spaces intact. Normalisation is deliberately not applied while typing —
/// showing a tidied value the user did not type would be putting words in
/// their mouth. The trim happens in `settled_draft`, at the moment the
/// value becomes a write.
#[test]
fn enter_with_an_answer_moves_to_the_next_row_without_writing() {
    let typed = type_str(&add_at(FormCursor::Field(AddField::Alias)), "  web-01  ");
    let advanced = step(&typed, key(KeyCode::Enter), Some(&web01()));

    let sequence = add_seq(&advanced.state);
    assert_eq!(sequence.cursor, FormCursor::Field(AddField::Host));
    assert_eq!(
        sequence.draft.alias, "  web-01  ",
        "the live draft shows what was typed, spaces and all"
    );
    assert_eq!(
        sequence.error, None,
        "and carries no complaint from the last row"
    );
    assert!(
        advanced.effects.is_empty(),
        "moving off a row writes nothing: {:?}",
        advanced.effects
    );

    // Fill the Host so the form can settle, and the trim shows up there.
    let with_host = type_str(&advanced.state, "10.0.0.4");
    let settled = manage::settled_draft(&add_seq(&with_host).draft)
        .expect("alias and host filled, so the draft settles");
    assert_eq!(
        settled.alias, "web-01",
        "the settled draft is trimmed — a leading space in an alias is a \
         typo, not content"
    );
}

/// Walking the whole map row by row, in the spec's order.
#[test]
fn the_map_walks_the_six_fields_the_spec_names() {
    let mut state = add_at(FormCursor::Field(AddField::Alias));

    state = type_str(&state, "web-01");
    state = step(&state, key(KeyCode::Enter), Some(&web01())).state;
    assert_eq!(cursor_of(&state), FormCursor::Field(AddField::Host));
    state = type_str(&state, "10.0.0.4");
    state = step(&state, key(KeyCode::Enter), Some(&web01())).state;
    assert_eq!(cursor_of(&state), FormCursor::Field(AddField::User));
    state = type_str(&state, "deploy");
    state = step(&state, key(KeyCode::Enter), Some(&web01())).state;
    assert_eq!(cursor_of(&state), FormCursor::Field(AddField::Port));
    state = type_str(&state, "2222");
    state = step(&state, key(KeyCode::Enter), Some(&web01())).state;
    assert_eq!(cursor_of(&state), FormCursor::Field(AddField::Key));
    state = type_str(&state, "~/.ssh/id_ed25519");
    state = step(&state, key(KeyCode::Enter), Some(&web01())).state;
    assert_eq!(cursor_of(&state), FormCursor::Field(AddField::Folder));

    assert_eq!(
        add_seq(&state).draft,
        ConnectionDraft {
            alias: "web-01".into(),
            host: "10.0.0.4".into(),
            user: "deploy".into(),
            port: "2222".into(),
            key_path: "~/.ssh/id_ed25519".into(),
            folder: String::new(),
        },
        "every row walked keeps what it was given, in order"
    );
}

/// A port that is not a number is a problem the map names where it was
/// typed. Silently falling back to 22 here would build a Connection that
/// connects to the wrong machine, and the user would only find out from
/// `ssh`.
#[test]
fn a_non_numeric_port_is_rejected_at_the_submit_row() {
    let typed = type_str(&at_port(), "ssh");

    assert_eq!(
        glyph_of(&typed, AddField::Port),
        RowGlyph::Invalid,
        "the Port row shows `!` for content that will not validate"
    );
    assert_eq!(
        problems_of(&typed),
        vec![("Port", PORT_REFUSAL.to_string())],
        "and names the field and the rule"
    );
    assert!(!add_seq(&typed).ready());

    let refused = submit(&typed);
    assert_eq!(
        error_of(&refused.state),
        Some(PORT_REFUSAL),
        "the ▶ row refuses with the same sentence"
    );
    assert!(
        refused.effects.is_empty(),
        "a refusal asks the driver for nothing: {:?}",
        refused.effects
    );
    assert!(
        matches!(refused.state.phase, Phase::Add(_)),
        "and the form stays open with what was typed still in it"
    );
}

#[test]
fn a_port_above_the_tcp_range_is_rejected() {
    let typed = type_str(&at_port(), "70000");

    assert_eq!(glyph_of(&typed, AddField::Port), RowGlyph::Invalid);
    assert_eq!(problems_of(&typed), vec![("Port", PORT_REFUSAL.to_string())]);
}

#[test]
fn port_zero_is_rejected() {
    let typed = type_str(&at_port(), "0");

    assert_eq!(glyph_of(&typed, AddField::Port), RowGlyph::Invalid);
    assert_eq!(problems_of(&typed), vec![("Port", PORT_REFUSAL.to_string())]);
}

#[test]
fn a_negative_port_is_rejected() {
    let typed = type_str(&at_port(), "-1");

    assert_eq!(glyph_of(&typed, AddField::Port), RowGlyph::Invalid);
    assert_eq!(problems_of(&typed), vec![("Port", PORT_REFUSAL.to_string())]);
}

/// The port is the one row that is optional *and* has a value when left
/// alone: the SSH default. Blank is not "absent" here — `Connection.port`
/// is a `u16`, there is no absent to settle to — so the settled draft
/// carries `22`.
///
/// The live row keeps the blank, though. The map shows what the user left
/// it as; the default is applied at the moment the value becomes a write.
#[test]
fn an_empty_port_settles_as_the_ssh_default() {
    let at_port = goto(&filled_add(), FormCursor::Field(AddField::Port));
    let cleared = backspace_n(&at_port, "2222".chars().count());

    assert_eq!(
        add_seq(&cleared).draft.port,
        "",
        "the live row keeps the blank the user left there"
    );
    assert_eq!(
        glyph_of(&cleared, AddField::Port),
        RowGlyph::Empty,
        "and it reads as an optional row left empty, not as a problem"
    );
    assert!(
        add_seq(&cleared).ready(),
        "a blank port must not block the submit: {:?}",
        add_seq(&cleared).problems()
    );

    let added = submit(&cleared);
    let draft = only_add(&added);
    assert_eq!(
        draft.port, "22",
        "the settled draft normalises the empty port to the SSH default"
    );
}

#[test]
fn a_valid_port_settles_as_typed() {
    let typed = type_str(&at_port(), "2222");

    assert_eq!(glyph_of(&typed, AddField::Port), RowGlyph::Valid);
    let added = submit(&typed);
    let draft = only_add(&added);
    assert_eq!(draft.port, "2222");
}

/// An optional row accepts empty, and what it settles to is **absent**,
/// not the empty string. The difference is not pedantry: `Connection`'s
/// `key_path` is an `Option<String>`, and a `Some("")` reaches `ssh` as
/// `-i ""` — an identity file that is a zero-length path.
#[test]
fn an_empty_key_is_accepted_and_settles_as_absent() {
    let at_key = goto(&filled_add(), FormCursor::Field(AddField::Key));
    let cleared = backspace_n(&at_key, "~/.ssh/id_ed25519".chars().count());

    assert_eq!(
        glyph_of(&cleared, AddField::Key),
        RowGlyph::Empty,
        "blank is an answer for an optional row: it reads as empty, not as needed"
    );
    assert!(add_seq(&cleared).ready());

    let added = submit(&cleared);
    let draft = only_add(&added);
    assert_eq!(
        draft.key_path, "",
        "the draft carries the empty answer; the store turns it into absent"
    );
}

#[test]
fn an_empty_folder_is_accepted_and_settles_as_absent() {
    let at_folder = goto(&filled_add(), FormCursor::Field(AddField::Folder));
    let cleared = backspace_n(&at_folder, "prod".chars().count());

    assert_eq!(glyph_of(&cleared, AddField::Folder), RowGlyph::Empty);
    assert!(add_seq(&cleared).ready());

    let added = submit(&cleared);
    let draft = only_add(&added);
    assert_eq!(draft.folder, "");
    assert_eq!(
        draft.key_path, "~/.ssh/id_ed25519",
        "the answered optional row is still there"
    );
}

/// A whitespace-only optional row is the same as a blank one: trimmed to
/// nothing, absent. A folder of `"  "` would render as `[  ]` in every
/// row of the list forever.
#[test]
fn a_whitespace_only_optional_field_settles_as_absent_too() {
    let at_folder = goto(&filled_add(), FormCursor::Field(AddField::Folder));
    let cleared = backspace_n(&at_folder, "prod".chars().count());
    let typed = type_str(&cleared, "   ");

    assert_eq!(
        glyph_of(&typed, AddField::Folder),
        RowGlyph::Empty,
        "three spaces in an optional row read as empty"
    );

    let added = submit(&typed);
    let draft = only_add(&added);
    assert_eq!(draft.folder, "", "and the settled draft trims them away");
}

#[test]
fn an_answered_optional_field_settles_as_itself() {
    let at_key = goto(&filled_add(), FormCursor::Field(AddField::Key));
    let cleared = backspace_n(&at_key, "~/.ssh/id_ed25519".chars().count());
    let typed = type_str(&cleared, "~/.ssh/id_work");

    let added = submit(&typed);
    let draft = only_add(&added);
    assert_eq!(draft.key_path, "~/.ssh/id_work");
}

/// The one refusal that names everything: Enter on the `▶` row over a form
/// with several bad rows puts **every** bad field on one line, in map
/// order, rather than making the user discover them one Enter at a time.
#[test]
fn enter_on_submit_with_invalid_fields_names_every_bad_field() {
    let at_port = goto(&add_at(FormCursor::Field(AddField::Alias)), FormCursor::Field(AddField::Port));
    let typed = type_str(&at_port, "99999");

    let refused = submit(&typed);

    assert_eq!(
        error_of(&refused.state),
        Some(
            [&format!("alias is required"), &format!("host is required"), PORT_REFUSAL]
                .join(JOIN)
                .as_str(),
        ),
        "the refusal must name all three bad rows at once, in map order"
    );
    assert!(
        refused.effects.is_empty(),
        "a refusal must not write anything: {:?}",
        refused.effects
    );
    assert!(
        matches!(refused.state.phase, Phase::Add(_)),
        "and the phase must be unchanged — the form stays open to be fixed"
    );
    assert_eq!(
        cursor_of(&refused.state),
        FormCursor::Submit,
        "the cursor stays on the row that was refused"
    );
}

/// A refusal is cleared by the next keystroke that could fix it, so a
/// mistake stops being reported the moment the user starts correcting it.
#[test]
fn a_refusal_is_cleared_by_the_next_keystroke() {
    let refused = submit(&add_at(FormCursor::Submit));
    assert!(error_of(&refused.state).is_some(), "setup");

    let at_alias = goto(&refused.state, FormCursor::Field(AddField::Alias));
    let typed = step(&at_alias, plain('w'), Some(&web01()));

    assert_eq!(
        error_of(&typed.state),
        None,
        "starting to fix the form must clear the complaint"
    );
}

/// The glyph decision, in its precedence order.
///
/// The frame draws these, and the ordering is the whole point: a cleared
/// required row is both `Needed` and `Changed`, and showing `Changed`
/// there would let a blocker look like an ordinary edit.
#[test]
fn the_glyph_precedence_shows_the_blocker_before_the_edit() {
    // Invalid content beats everything.
    let bad_port = type_str(&at_port(), "ssh");
    assert_eq!(glyph_of(&bad_port, AddField::Port), RowGlyph::Invalid);

    // Empty-and-required beats Changed: clearing the Host of a live
    // Connection is both, and the one that blocks the save wins.
    let cleared_host = backspace_n(
        &goto(
            &edit_state(&web01()),
            FormCursor::Field(AddField::Host),
        ),
        "10.0.0.4".chars().count(),
    );
    assert_eq!(
        glyph_of(&cleared_host, AddField::Host),
        RowGlyph::Needed,
        "a cleared required row must not read as a harmless change"
    );

    // A valid edit of a filled row reads as Changed.
    let changed_alias = type_str(
        &goto(&edit_state(&web01()), FormCursor::Field(AddField::Alias)),
        "-x",
    );
    assert_eq!(
        glyph_of(&changed_alias, AddField::Alias),
        RowGlyph::Changed,
        "a row that differs from the stored Connection wears ●"
    );

    // An untouched filled row is Valid; an untouched empty optional is Empty.
    let fresh = edit_state(&web01());
    assert_eq!(glyph_of(&fresh, AddField::Host), RowGlyph::Valid);
    assert_eq!(glyph_of(&fresh, AddField::Key), RowGlyph::Empty);

    // And the glyphs themselves are the five the design names.
    assert_eq!(RowGlyph::Valid.char(), '\u{2713}');
    assert_eq!(RowGlyph::Changed.char(), '\u{25cf}');
    assert_eq!(RowGlyph::Needed.char(), '\u{25cb}');
    assert_eq!(RowGlyph::Invalid.char(), '!');
    assert_eq!(RowGlyph::Empty.char(), '\u{b7}');
}

// ─────────────────────────────────────────────────────────────────────────────
// Slice 9 — the note reports what the store actually did, and backing out
// writes nothing
// ─────────────────────────────────────────────────────────────────────────────

/// The `▶` row asks for the write. It does not claim the write happened.
/// The state machine owns no disk, and `◇ added` is a claim about
/// `connections.json`.
#[test]
fn finishing_the_sequence_asks_for_the_add_without_claiming_it_happened() {
    let finalised = submit(&filled_add());

    assert_eq!(
        finalised.state.phase,
        Phase::List,
        "the form is over; the frame is back on the list"
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
    let finalised = submit(&filled_add());

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
    let finalised = submit(&filled_add());

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

/// Backing out of the first row is backing out of the form. Nothing was
/// ever written — no `Effect::Add` was ever emitted — and the note says
/// so, because the question the user has at that moment is whether the
/// half-filled form got saved.
#[test]
fn abandoning_the_sequence_mid_way_asks_for_nothing() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));
    let typed = step(&adding.state, plain('w'), Some(&web01()));

    let abandoned = step(&typed.state, key(KeyCode::Esc), Some(&web01()));

    assert_eq!(
        abandoned.state.phase,
        Phase::List,
        "backing off the first row leaves the form, not a row before it"
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

/// Going back and then forward again must not lose what a row holds.
///
/// The stepped add had to *re-seed* the line from the stored answer on
/// every move, and getting that wrong wiped a settled field on a bare
/// Enter. The map has no line to re-seed: the row *is* the draft, so
/// arriving at a row from either direction shows the same truth, and a
/// bare Enter on the way back cannot overwrite anything.
#[test]
fn going_back_and_forward_again_keeps_every_rows_own_value() {
    let at_folder = goto(&filled_add(), FormCursor::Field(AddField::Folder));

    // Back twice: Folder → Key → Port.
    let at_key_again = step(&at_folder, key(KeyCode::Esc), Some(&web01())).state;
    let at_port_again = step(&at_key_again, key(KeyCode::Esc), Some(&web01())).state;
    assert_eq!(cursor_of(&at_port_again), FormCursor::Field(AddField::Port));
    assert_eq!(add_seq(&at_port_again).draft.port, "2222");

    // Forward again over Key without touching it.
    let forward = step(&at_port_again, key(KeyCode::Enter), Some(&web01()));
    assert_eq!(
        cursor_of(&forward.state),
        FormCursor::Field(AddField::Key),
        "setup"
    );
    assert_eq!(
        add_seq(&forward.state).draft.key_path,
        "~/.ssh/id_ed25519",
        "the row must arrive holding what it already held, not an empty slot \
         waiting to overwrite it"
    );

    // And walking on to the submit still carries the Key.
    let added = submit(&forward.state);
    let draft = only_add(&added);
    assert_eq!(
        draft.key_path, "~/.ssh/id_ed25519",
        "walking back and forth must not destroy the answered Key"
    );
}

/// Walking the whole way back from the bottom ends the same way, and writes
/// nothing at any point on the way.
#[test]
fn walking_all_the_way_back_writes_nothing() {
    let mut state = goto(&filled_add(), FormCursor::Field(AddField::Folder));

    // Folder → Key → Port → User → Host → Alias: five rows up, still in
    // the form at each one.
    for _ in 0..5 {
        state = step(&state, key(KeyCode::Esc), Some(&web01())).state;
        assert!(matches!(state.phase, Phase::Add(_)), "still walking back");
    }
    assert_eq!(cursor_of(&state), FormCursor::Field(AddField::Alias));

    let last = step(&state, key(KeyCode::Esc), Some(&web01()));
    assert_eq!(last.state.phase, Phase::List);
    assert_eq!(last.state.trace, Some(Trace::AddAbandoned));
    assert!(last.effects.is_empty());
}

/// Ctrl+C in the middle of a form is the frame's cancel, not the row's:
/// it leaves, and it leaves nothing behind.
#[test]
fn ctrl_c_mid_sequence_cancels_the_frame_without_writing() {
    let cancelled = step(&filled_add(), ctrl('c'), Some(&web01()));

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
// Ctrl+E — the same map, seeded from a Connection (#37)
//
// `Ctrl+E` is not a second editor. It opens the six-row map `Ctrl+A`
// opens, with the same cursor and the same `▶` row; the differences are
// that the rows arrive filled, the button says *Save* and names the
// Connection, and there is a baseline to diff against so the changed rows
// wear `●` and the save is reviewable before it happens.
//
// This deliberately departs from #31 story 26's "change exactly one
// field". The old editor committed on every Enter, so a two-field
// correction wrote the file twice and could not be abandoned halfway
// through. Here Enter advances and the `▶` row commits, so one save can
// carry several changed fields in one write.
// ─────────────────────────────────────────────────────────────────────────────

/// `Ctrl+E` opens the map on the Connection the chord was pressed over,
/// with every row pre-filled from what that Connection actually holds.
///
/// The pre-fill is what makes this an *edit* rather than a second add: the
/// user sees the thing they are changing. Starting with empty rows would
/// ask them to retype a Connection they already have.
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
    assert_eq!(
        editor.cursor,
        FormCursor::Field(AddField::Alias),
        "the cursor starts at the top, not on the field the user is most \
         likely to change — a predictable row beats a clever one"
    );
    assert_eq!(
        editor.draft.alias, "web-01",
        "every row must arrive holding the field's current value"
    );
    assert_eq!(editor.draft.host, "10.0.0.4");
    assert_eq!(editor.draft.user, "deploy");
    assert_eq!(editor.draft.port, "22");
    assert_eq!(editor.draft.folder, "prod");
    assert_eq!(
        editor.baseline, editor.draft,
        "the baseline is the untouched copy the diff is taken against"
    );
    assert!(
        editor.changed().is_empty(),
        "and a freshly opened edit has changed nothing: {:?}",
        editor.changed()
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
    assert_eq!(step.state.phase, Phase::List);
}

/// The editor is scoped to the target captured at the chord, not to the
/// cursor. Arrowing across six rows and the `▶` row must not be able to
/// move the write onto a neighbour — the same rule that stops a drifting
/// cursor moving a delete.
#[test]
fn the_edit_target_is_captured_at_the_chord_and_never_moves() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));

    // Walk the whole map round twice, with the caller reporting a
    // different selection the entire time.
    let mut state = armed.state.clone();
    for _ in 0..14 {
        state = step(&state, key(KeyCode::Tab), Some(&web02())).state;
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
/// A stale `◇ deleted` parked above "which row am I editing?" reads as
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
    assert_eq!(editor.draft.alias, "web-01db-01");
    assert_eq!(
        state.query, "",
        "the keystrokes belong to the row; the filter the user returns to is \
         untouched"
    );
}

/// Moving the cursor shows the row it lands on, not the row it left.
///
/// This is what makes moving between rows safe: the row always shows the
/// truth about the field it names, so a user who arrows from Alias to Port
/// cannot be looking at a hostname while about to write a port.
#[test]
fn moving_the_field_shows_that_fields_actual_value() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));

    let on_host = step(&armed.state, key(KeyCode::Down), Some(&web01())).state;

    let Phase::Edit(editor) = &on_host.phase else {
        panic!("still editing, got {:?}", on_host.phase);
    };
    assert_eq!(editor.cursor, FormCursor::Field(AddField::Host));
    let rows = editor.rows();
    assert_eq!(
        row(&rows, AddField::Host).value.as_deref(),
        Some("10.0.0.4"),
        "the row must show the Host, not the Alias it was showing a keystroke ago"
    );
    assert!(
        row(&rows, AddField::Host).focused,
        "and the frame must be able to tell which row that is"
    );
    assert!(
        !row(&rows, AddField::Alias).focused,
        "with the row it left no longer marked focused"
    );
}

#[test]
fn the_field_selector_walks_the_six_fields_in_the_specs_order() {
    let mut state = step(&ManageState::new(), ctrl('e'), Some(&web01())).state;
    let mut seen = vec![cursor_of(&state)];

    for _ in 0..6 {
        state = step(&state, key(KeyCode::Down), Some(&web01())).state;
        seen.push(cursor_of(&state));
    }

    assert_eq!(
        seen,
        vec![
            FormCursor::Field(AddField::Alias),
            FormCursor::Field(AddField::Host),
            FormCursor::Field(AddField::User),
            FormCursor::Field(AddField::Port),
            FormCursor::Field(AddField::Key),
            FormCursor::Field(AddField::Folder),
            FormCursor::Submit,
        ],
        "the editor walks the same six fields, in the same order, the add map \
         walks, ending on the `▶` row"
    );
    assert_eq!(
        cursor_of(&state),
        FormCursor::Submit,
        "`↓` saturates on the `▶` row rather than wrapping"
    );
}

/// The selector wraps at both ends on the travel keys.
///
/// A `BackTab` that silently dies at the first field reads as a broken
/// keybinding on a rail that has already promised the keys move the row.
#[test]
fn the_field_selector_wraps_at_both_ends() {
    let at_alias = step(&ManageState::new(), ctrl('e'), Some(&web01())).state;

    let back = step(&at_alias, key(KeyCode::BackTab), Some(&web01())).state;
    assert_eq!(
        cursor_of(&back),
        FormCursor::Submit,
        "BackTab off the first row must land on the `▶` row, not nowhere"
    );

    let forward = step(&back, key(KeyCode::Tab), Some(&web01())).state;
    assert_eq!(
        cursor_of(&forward),
        FormCursor::Field(AddField::Alias),
        "and Tab off the `▶` row must come back round to the first"
    );
}

/// Enter on a field in the editor advances; it never commits.
///
/// The old editor wrote the file on every Enter, which is what made a
/// two-field correction two writes. Here Enter is the same movement key it
/// is in the add map, and the only write is the `▶` row.
#[test]
fn enter_on_a_field_in_the_editor_advances_and_writes_nothing() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));

    let advanced = step(&armed.state, key(KeyCode::Enter), Some(&web01()));

    assert_eq!(
        cursor_of(&advanced.state),
        FormCursor::Field(AddField::Host),
        "Enter moves down one row"
    );
    assert!(
        advanced.effects.is_empty(),
        "and commits nothing: the only write in this form is the ▶ row: {:?}",
        advanced.effects
    );
}

/// **Edit with no changes refuses, and says why.**
///
/// An add with nothing filled is simply not ready. An edit that has touched
/// nothing has answered a question nobody asked, and silently writing the
/// file back would let the user believe something was saved when the
/// Connection on disk is byte-identical to the one they started from.
#[test]
fn an_edit_with_no_changes_is_refused_as_nothing_to_save() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));

    let refused = submit(&armed.state);

    assert_eq!(
        error_of(&refused.state),
        Some("nothing to save"),
        "the ▶ Save row must refuse an untouched form by name"
    );
    assert!(
        refused.effects.is_empty(),
        "and must not write the file back unchanged: {:?}",
        refused.effects
    );
    assert!(
        matches!(refused.state.phase, Phase::Edit(_)),
        "the editor stays open with its target intact"
    );
    assert_eq!(
        edit_seq(&refused.state).target,
        web01(),
        "the refusal must not lose the Connection being edited"
    );
}

/// Typing whitespace into an empty optional row is not a change.
///
/// Compared trimmed on purpose: if it counted, the `▶ Save` row would light
/// up for a save that writes exactly what is already there.
#[test]
fn whitespace_in_an_untouched_optional_row_is_still_nothing_to_save() {
    let no_key = Connection {
        key_path: None,
        ..web01()
    };
    let armed = step(&ManageState::new(), ctrl('e'), Some(&no_key));
    let at_key = goto(&armed.state, FormCursor::Field(AddField::Key));
    let typed = type_str(&at_key, "   ");

    assert!(
        edit_seq(&typed).changed().is_empty(),
        "three spaces in an empty optional row are not a change: {:?}",
        edit_seq(&typed).changed()
    );

    let refused = submit(&typed);
    assert_eq!(error_of(&refused.state), Some("nothing to save"));
    assert!(refused.effects.is_empty());
}

/// **Several changed fields, one write.**
///
/// The whole point of replacing the per-Enter commit: a correction that
/// touches the host and the port used to be two writes with a window
/// between them in which the Connection was half-changed and could not be
/// backed out of. Here the `▶` row carries every changed field in a single
/// `Effect::Update`.
#[test]
fn an_edit_changing_several_fields_is_one_update_carrying_all_of_them() {
    let armed = edit_state(&web01());

    let on_host = goto(&armed, FormCursor::Field(AddField::Host));
    let on_host = backspace_n(&on_host, "10.0.0.4".chars().count());
    let on_host = type_str(&on_host, "10.0.0.99");
    let on_port = goto(&on_host, FormCursor::Field(AddField::Port));
    let on_port = backspace_n(&on_port, "22".chars().count());
    let on_port = type_str(&on_port, "2222");
    let on_user = goto(&on_port, FormCursor::Field(AddField::User));
    let on_user = backspace_n(&on_user, "deploy".chars().count());
    let changed = type_str(&on_user, "root");

    assert_eq!(
        edit_seq(&changed).changed(),
        vec![AddField::Host, AddField::User, AddField::Port],
        "three rows changed, reported in map order"
    );
    assert!(edit_seq(&changed).ready());

    let saved = submit(&changed);

    assert_eq!(
        saved.effects.len(),
        1,
        "three changed fields must be ONE write, not three: {:?}",
        saved.effects
    );
    let (target, draft) = only_update(&saved);
    assert_eq!(target, &web01(), "the request names the target");
    assert_eq!(draft.host, "10.0.0.99", "the host change is carried");
    assert_eq!(draft.user, "root", "the user change is carried");
    assert_eq!(draft.port, "2222", "the port change is carried");
    assert_eq!(draft.alias, "web-01", "and the untouched alias comes along");
    assert_eq!(draft.folder, "prod", "and the untouched folder too");
}

/// The id travels with the edit.
///
/// A write that minted a fresh id would orphan every reference to the
/// Connection the user was editing and leave the old row in the list.
#[test]
fn the_edit_carries_the_targets_id_not_a_new_one() {
    let armed = edit_state(&web01());
    let renamed = type_str(
        &goto(&armed, FormCursor::Field(AddField::Alias)),
        "-renamed",
    );

    let saved = submit(&renamed);

    let (target, draft) = only_update(&saved);
    assert_eq!(target.id, "id-web-01");
    assert_eq!(
        draft.alias, "web-01-renamed",
        "the draft is the whole Connection, so the store can keep the id"
    );
}

/// The editor validates through the same `settled_draft` the add map uses,
/// so a field cannot have two different rules depending on which chord
/// opened it.
#[test]
fn the_editor_applies_the_same_per_field_validation_as_the_add_sequence() {
    let armed = edit_state(&web01());

    // Out of range, alongside a real change so the refusal is about the
    // port and not about there being nothing to save.
    let on_alias = type_str(&goto(&armed, FormCursor::Field(AddField::Alias)), "x");
    let on_port = goto(&on_alias, FormCursor::Field(AddField::Port));
    let cleared = backspace_n(&on_port, "22".chars().count());
    let bad = type_str(&cleared, "99999");

    let refused = submit(&bad);

    assert_eq!(
        error_of(&refused.state),
        Some(PORT_REFUSAL),
        "the same sentence the add map gives"
    );
    assert!(
        refused.effects.is_empty(),
        "a refused field must not write: {:?}",
        refused.effects
    );

    // Empty: the SSH default, the same answer the add map gives.
    let cleared = backspace_n(&bad, "99999".chars().count());
    let settled = submit(&cleared);
    let (_, draft) = only_update(&settled);
    assert_eq!(draft.port, "22");
}

#[test]
fn a_refused_edit_keeps_the_typed_text_and_the_target() {
    let armed = edit_state(&web01());

    // Blank a required field and try to save.
    let cleared = backspace_n(
        &goto(&armed, FormCursor::Field(AddField::Alias)),
        "web-01".chars().count(),
    );
    let refused = submit(&cleared);

    let Phase::Edit(editor) = &refused.state.phase else {
        panic!("must still be editing, got {:?}", refused.state.phase);
    };
    assert_eq!(editor.target, web01(), "the target survives the refusal");
    assert_eq!(editor.draft.alias, "", "what the user left stays where it is");
    assert_eq!(
        editor.error.as_deref(),
        Some("alias is required"),
        "the refusal names the field"
    );
    assert!(refused.effects.is_empty());
}

#[test]
fn esc_abandons_the_edit_and_promises_nothing_was_saved() {
    let armed = edit_state(&web01());
    let typed = type_str(&armed, "-typo");

    let step = step(&typed, key(KeyCode::Esc), Some(&web01()));

    assert_eq!(step.state.phase, Phase::List);
    assert_eq!(step.state.trace, Some(Trace::EditAbandoned));
    assert!(
        step.effects.is_empty(),
        "abandoning an edit must not write: {:?}",
        step.effects
    );
}

/// Esc partway down the edit map moves up one row rather than abandoning,
/// so a user correcting the Port can back off a row without losing the
/// whole edit.
#[test]
fn esc_in_the_editor_moves_up_one_row_before_it_abandons() {
    let on_port = goto(&edit_state(&web01()), FormCursor::Field(AddField::Port));

    let up = step(&on_port, key(KeyCode::Esc), Some(&web01()));
    assert_eq!(
        cursor_of(&up.state),
        FormCursor::Field(AddField::User),
        "one Esc is one row up"
    );
    assert!(matches!(up.state.phase, Phase::Edit(_)));

    let up = step(&up.state, key(KeyCode::Esc), Some(&web01()));
    assert_eq!(cursor_of(&up.state), FormCursor::Field(AddField::Host));

    let up = step(&up.state, key(KeyCode::Esc), Some(&web01()));
    assert_eq!(cursor_of(&up.state), FormCursor::Field(AddField::Alias));

    let abandoned = step(&up.state, key(KeyCode::Esc), Some(&web01()));
    assert_eq!(abandoned.state.phase, Phase::List);
    assert_eq!(abandoned.state.trace, Some(Trace::EditAbandoned));
}

#[test]
fn ctrl_c_inside_the_editor_cancels_the_frame_without_writing() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));

    let step = step(&armed.state, ctrl('c'), Some(&web01()));

    assert_eq!(step.effects, vec![Effect::Exit(InlineOutcome::Cancelled)]);
}

/// The list's own cursor cannot move while the editor is open.
///
/// The target is captured at the chord, so there is nothing for the list
/// cursor to change — and a stray arrow that moved it mid-edit would put
/// the write somewhere the user never pointed. `↑`/`↓` *do* move the form
/// row now; what must not move is `selection` and the Connection being
/// edited.
#[test]
fn the_list_cursor_cannot_move_while_the_editor_is_open() {
    let armed = ManageState {
        selection: 3,
        ..edit_state(&web01())
    };

    for code in [KeyCode::Up, KeyCode::Down, KeyCode::Tab, KeyCode::BackTab] {
        let step = step(&armed, key(code), Some(&web02()));

        assert_eq!(
            step.state.selection, 3,
            "{code:?} must not move the list cursor while editing"
        );
        assert_eq!(
            edit_seq(&step.state).target,
            web01(),
            "{code:?} must not move the edit onto another Connection"
        );
        assert!(step.effects.is_empty());
    }
}

/// Clearing an optional field on the edit path makes it *absent*, not an
/// empty string.
///
/// A `key_path: Some("")` would reach ssh as `-i ""`. The same rule the
/// add map enforces, reached through the same `settled_draft`.
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

    let on_key = goto(&edit_state(&with_key), FormCursor::Field(AddField::Key));
    let cleared = backspace_n(&on_key, "~/.ssh/id_ed25519".chars().count());
    let step = submit(&cleared);

    let (_, draft) = only_update(&step);
    assert_eq!(
        draft.key_path, "",
        "the draft carries the empty answer; the store turns it into absent"
    );
}

/// The edit is the same map as the add, not a second editor with its own
/// shape.
///
/// Story 26 asked for a small correction to take *one* step, and the old
/// answer was a single-field editor. The new answer is the same thing by
/// different means: the add's own six-row map, seeded, with one `▶` row
/// that commits. Two flows that look alike and behave differently are the
/// thing users get wrong, so the shared shape is the promise — and one
/// Enter on `▶` is still the whole save.
#[test]
fn the_edit_is_the_same_map_as_the_add_with_one_submit_row() {
    let adding = add_at(FormCursor::Field(AddField::Alias));
    let editing = edit_state(&web01());

    let add_rows: Vec<(AddField, &'static str)> = add_seq(&adding)
        .rows()
        .iter()
        .map(|r| (r.field, r.label))
        .collect();
    let edit_rows: Vec<(AddField, &'static str)> = edit_seq(&editing)
        .rows()
        .iter()
        .map(|r| (r.field, r.label))
        .collect();

    assert_eq!(
        add_rows, edit_rows,
        "both chords open the same six rows, labelled the same, in the same order"
    );
    assert_eq!(
        cursor_of(&adding),
        cursor_of(&editing),
        "and both start the cursor on the same row"
    );

    let saved = submit(&type_str(
        &goto(&editing, FormCursor::Field(AddField::Alias)),
        "x",
    ));
    assert_eq!(
        saved.effects.len(),
        1,
        "one Enter on the ▶ row is the whole edit"
    );
    assert!(matches!(saved.effects[0], Effect::Update { .. }));
    assert!(
        !matches!(saved.state.phase, Phase::Add(_)),
        "Ctrl+E must never open the add flow"
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

// ─────────────────────────────────────────────────────────────────────────────
// Slice 12 — the first-run import offer (#38)
//
// The offer is the one place the manage frame asks the user to authorize a
// write they did not type out field by field, so it is held to the same
// discipline as the delete confirm: the keystroke is an *ask*, and the
// `◇ imported` note is a claim about `connections.json` that only the
// store can earn. The tests below keep those two halves apart.
// ─────────────────────────────────────────────────────────────────────────────

/// The path the ticket's worked example offers to import from.
const OFFER_PATH: &str = "/home/deploy/.ssh/config";

/// The frame sitting on the first-run offer: twelve importable
/// Connections found at [`OFFER_PATH`], nothing written yet.
///
/// The list is empty here by definition — that is what makes it a
/// first run — so every keystroke below is driven with `selected: None`.
/// That is the point: the offer cannot depend on a cursor, because there
/// is nothing to point at.
fn offered() -> ManageState {
    ManageState {
        phase: Phase::ConfirmImport {
            count: 12,
            path: OFFER_PATH.to_string(),
        },
        ..ManageState::new()
    }
}

/// `y` asks for the import and returns to the list — and asks for *one*
/// import, of the path the offer named.
///
/// The count travels in the phase rather than the effect because the
/// count is what the scan found, not what the write will deliver; the
/// effect names only the file, and the report that comes back says how
/// many arrived.
#[test]
fn answering_y_at_the_import_offer_asks_for_exactly_one_import_of_the_offered_path() {
    let step = step(&offered(), plain('y'), None);

    assert_eq!(
        step.effects,
        vec![Effect::Import {
            path: OFFER_PATH.to_string()
        }],
        "`y` must ask for exactly one import, of the path the offer named"
    );
    assert_eq!(
        step.state.phase,
        Phase::List,
        "the offer is over once it has been answered"
    );
    assert_eq!(
        step.state.trace, None,
        "the keystroke must not pre-commit an `imported` note. The state \
         machine owns no disk: the claim is earned by the store's answer, \
         folded in by `settle_import`"
    );
}

/// Shift is allowed on the answer, exactly as it is on the delete
/// confirm: `Y` is the same yes, and a user with the shift down must not
/// be silently read as having typed some other printable.
#[test]
fn a_capital_y_answers_the_import_offer_like_a_lowercase_one() {
    let step = step(&offered(), plain('Y'), None);

    assert_eq!(
        step.effects,
        vec![Effect::Import {
            path: OFFER_PATH.to_string()
        }],
        "`Y` must ask for the same import `y` does"
    );
    assert_eq!(step.state.trace, None);
}

/// The effect carries the path it was offered with, not a path it
/// resolved for itself.
///
/// This is the whole reason the path lives in the phase: the user was
/// shown one file, so the write has to be about that file. An effect
/// that re-derived `~/.ssh/config` at the driver could import something
/// the user never agreed to.
#[test]
fn the_import_effect_carries_the_path_the_offer_was_made_at() {
    let elsewhere = ManageState {
        phase: Phase::ConfirmImport {
            count: 3,
            path: "/etc/ssh/ssh_config".into(),
        },
        ..ManageState::new()
    };

    let step = step(&elsewhere, plain('y'), None);

    assert_eq!(
        step.effects,
        vec![Effect::Import {
            path: "/etc/ssh/ssh_config".into()
        }],
        "the import must name the file the user was shown, not the default one"
    );
}

/// `n` declines, and a decline writes nothing.
///
/// The trace is the whole point of the arm: the user needs to see that
/// the offer was heard and refused, rather than the empty frame silently
/// carrying on as if nothing had been asked.
#[test]
fn answering_n_at_the_import_offer_declines_and_writes_nothing() {
    for ch in ['n', 'N'] {
        let step = step(&offered(), plain(ch), None);

        assert!(
            step.effects.is_empty(),
            "`{ch}` must ask for nothing: {:?}",
            step.effects
        );
        assert_eq!(
            step.state.phase,
            Phase::List,
            "and must put the frame back on the list"
        );
        assert_eq!(
            step.state.trace,
            Some(Trace::ImportDeclined),
            "with the decline left as the note"
        );
    }
}

/// Esc abandons the offer the same way `n` does: nothing written, the
/// decline on the record.
#[test]
fn esc_at_the_import_offer_declines_it() {
    let step = step(&offered(), key(KeyCode::Esc), None);

    assert!(
        step.effects.is_empty(),
        "Esc must ask for nothing: {:?}",
        step.effects
    );
    assert_eq!(step.state.phase, Phase::List);
    assert_eq!(step.state.trace, Some(Trace::ImportDeclined));
}

/// The safety rule the delete confirm runs on, restated for the offer:
/// a user who carries on typing is searching, not importing.
///
/// The offer appears unasked the first time `sshm manage` opens on an
/// empty set. A user who never meant to answer it and starts typing a
/// filter must end up with a filtered list and a declined import — not
/// with twelve Connections they did not ask for.
#[test]
fn a_stray_printable_at_the_import_offer_declines_it_and_filters() {
    for ch in ['w', 'd', 'x', 'a', 'e', ' '] {
        let step = step(&offered(), plain(ch), None);

        assert!(
            step.effects.is_empty(),
            "`{ch:?}` must not ask for an import: {:?}",
            step.effects
        );
        assert_eq!(
            step.state.phase,
            Phase::List,
            "`{ch:?}` must leave the offer"
        );
        assert_eq!(
            step.state.query,
            ch.to_string(),
            "and the character must land in the filter rather than be wasted"
        );
        assert_eq!(step.state.trace, Some(Trace::ImportDeclined));
    }
}

/// Movement is inert while the offer is open, as it is at the delete
/// confirm. There is nothing to move onto — the set is empty — and a key
/// that silently died would read as a broken keybinding, so the rule is
/// simply that the offer is untouched.
#[test]
fn movement_does_not_happen_while_the_import_offer_is_open() {
    for code in [
        KeyCode::Down,
        KeyCode::Up,
        KeyCode::Enter,
        KeyCode::Backspace,
        KeyCode::Left,
        KeyCode::Right,
    ] {
        let step = step(&offered(), key(code), None);

        assert_eq!(
            step.state,
            offered(),
            "{code:?} must leave the offer exactly as it was"
        );
        assert!(step.effects.is_empty(), "{code:?} must ask for nothing");
    }
}

/// Ctrl+C at the offer is a cancel, not an answer.
///
/// It must not import, and it must not be reported as a decline either:
/// the frame is leaving, and the exit is the only thing the driver is
/// asked to do.
#[test]
fn ctrl_c_at_the_import_offer_cancels_without_importing() {
    let step = step(&offered(), ctrl('c'), None);

    assert_eq!(
        step.effects,
        vec![Effect::Exit(InlineOutcome::Cancelled)],
        "Ctrl+C must leave the frame and ask for nothing else"
    );
}

/// The classification of `Store::import_ssh_config`'s answer is the
/// whole contract between the driver and this module, so it is pinned
/// here rather than left to be re-derived at the call site.
///
/// A report with failures alongside imports is still `Imported`: nine
/// Connections really arrived, and the three that did not are carried as
/// a count so the note can say `3 skipped` rather than hiding them.
#[test]
fn the_import_report_classifies_into_the_two_outcomes() {
    assert_eq!(
        ImportOutcome::from_report(Ok(ImportReport {
            imported: 12,
            failures: vec![],
        })),
        ImportOutcome::Imported {
            imported: 12,
            failed: 0
        },
        "a clean report is a clean import"
    );

    assert_eq!(
        ImportOutcome::from_report(Ok(ImportReport {
            imported: 9,
            failures: vec![
                "wildcard host \"*.example.com\"".into(),
                "wildcard host \"test?\"".into(),
                "wildcard host \"*.lan\"".into(),
            ],
        })),
        ImportOutcome::Imported {
            imported: 9,
            failed: 3
        },
        "a partial import is still an import, with the failures counted"
    );

    assert_eq!(
        ImportOutcome::from_report(Err("disk on fire".into())),
        ImportOutcome::Failed("disk on fire".into()),
        "and a store that refused is a failure, not an import of nothing"
    );
}

/// A real import earns the note, and the frame comes back showing the
/// list that was written.
///
/// The filter is cleared and the cursor goes to the top for the same
/// reason `settle_add` does it: the user just said yes to twelve new
/// Connections, and a stale filter left over from before the offer would
/// hide every one of them under `No matches` while a note above claimed
/// they had arrived.
#[test]
fn an_import_the_store_wrote_earns_the_imported_note() {
    let stale = ManageState {
        query: "web".into(),
        selection: 7,
        ..offered()
    };
    let answered = step(&stale, plain('y'), None);

    let settled = manage::settle_import(
        &answered.state,
        ImportOutcome::Imported {
            imported: 12,
            failed: 0,
        },
    )
    .expect("an import the store performed settles onto the list");

    assert_eq!(
        settled.trace,
        Some(Trace::Imported {
            imported: 12,
            failed: 0
        })
    );
    assert_eq!(settled.phase, Phase::List);
    assert_eq!(
        settled.query, "",
        "the filter must be cleared so the imported Connections are visible"
    );
    assert_eq!(settled.selection, 0, "and the cursor starts at the top");
}

/// The partial case keeps both numbers on the note.
///
/// `◇ imported 9 connections, 3 skipped` rather than the clean wording:
/// a user who was promised an import is owed to know which half did not
/// show up, and a note that rounded three failures away would be a small
/// lie about their file.
#[test]
fn a_partial_import_reports_both_halves() {
    let answered = step(&offered(), plain('y'), None);

    let settled = manage::settle_import(
        &answered.state,
        ImportOutcome::Imported {
            imported: 9,
            failed: 3,
        },
    )
    .expect("a partial import is still a real import");

    assert_eq!(
        settled.trace,
        Some(Trace::Imported {
            imported: 9,
            failed: 3
        })
    );
}

/// A store that refused the import collapses the frame rather than
/// leaving a note on it.
///
/// Same rule as a refused add or delete: a live list the store cannot
/// vouch for is worse than no list, and the frame has no honest thing to
/// show underneath a failed write.
#[test]
fn an_import_the_store_refused_collapses_the_frame() {
    let answered = step(&offered(), plain('y'), None);

    let outcome = manage::settle_import(
        &answered.state,
        ImportOutcome::Failed("Permission denied (os error 13)".into()),
    );

    assert_eq!(
        outcome,
        Err("Permission denied (os error 13)".into()),
        "the store's own message must survive to be reported, not be swallowed"
    );
    assert!(
        !matches!(answered.state.trace, Some(Trace::Imported { .. })),
        "and nothing on the pre-settle state may read as imported: {:?}",
        answered.state.trace
    );
}

/// A declined offer never reaches the settle path, so it can never be
/// dressed up as an import.
///
/// The decline is terminal at the step: no effect leaves, which means
/// `settle_import` is never called with anything about it. Pinned here
/// because the honest note (`import declined`) and the earned note
/// (`imported 12 connections`) must never be interchangeable.
#[test]
fn a_declined_offer_leaves_nothing_for_the_settle_path_to_claim() {
    let declined = step(&offered(), key(KeyCode::Esc), None);

    assert!(declined.effects.is_empty());
    assert_eq!(
        declined.state.trace,
        Some(Trace::ImportDeclined),
        "the decline is the note, not a stand-in for an import"
    );
    assert!(
        !matches!(declined.state.trace, Some(Trace::Imported { .. })),
        "and it must not read as an import: {:?}",
        declined.state.trace
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test helpers
// ─────────────────────────────────────────────────────────────────────────────

/// The add map, opened with `Ctrl+A` and with the cursor parked on `at`.
fn add_at(at: FormCursor) -> ManageState {
    let opened = step(&ManageState::new(), ctrl('a'), Some(&web01()));
    goto(&opened.state, at)
}

/// The add map with only the two required rows filled, cursor on Folder.
///
/// The fixture for tests that want a form the `▶` row will accept while
/// still holding something specific to prove about a row the required
/// fields say nothing about — the User row, chiefly. Deriving those tests
/// from a blank map instead would have them refused for reasons that have
/// nothing to do with what they are testing.
fn add_with_required_filled() -> ManageState {
    let mut state = add_at(FormCursor::Field(AddField::Alias));
    state = type_str(&state, "web-03");
    state = goto(&state, FormCursor::Field(AddField::Host));
    state = type_str(&state, "10.0.0.7");
    assert_eq!(
        add_seq(&state).draft.user, "",
        "setup: the User row must start empty for the test to prove anything"
    );
    state
}

/// The edit map opened over `conn`.
fn edit_state(conn: &Connection) -> ManageState {
    step(&ManageState::new(), ctrl('e'), Some(conn)).state
}

/// The add map with every one of the six rows filled, cursor on Folder.
///
/// Deliberately a *valid* form, so a test that wants a refusal has to say
/// which row it broke rather than inheriting one from the fixture.
fn filled_add() -> ManageState {
    let rows: [(AddField, &str); 6] = [
        (AddField::Alias, "web-03"),
        (AddField::Host, "10.0.0.7"),
        (AddField::User, "deploy"),
        (AddField::Port, "2222"),
        (AddField::Key, "~/.ssh/id_ed25519"),
        (AddField::Folder, "prod"),
    ];

    let mut state = add_at(FormCursor::Field(AddField::Alias));
    for (field, value) in rows {
        state = goto(&state, FormCursor::Field(field));
        state = type_str(&state, value);
    }
    assert!(
        add_seq(&state).ready(),
        "the fixture must be a form the ▶ row would accept: {:?}",
        add_seq(&state).problems()
    );
    state
}

/// The add form with Alias, Host and User answered, cursor on Port.
fn at_port() -> ManageState {
    let mut state = add_at(FormCursor::Field(AddField::Alias));
    state = type_str(&state, "web-01");
    state = goto(&state, FormCursor::Field(AddField::Host));
    state = type_str(&state, "10.0.0.4");
    state = goto(&state, FormCursor::Field(AddField::User));
    state = type_str(&state, "deploy");
    goto(&state, FormCursor::Field(AddField::Port))
}

/// Move the form cursor to `target` using the saturating arrows, which
/// reach every row from every other row.
fn goto(state: &ManageState, target: FormCursor) -> ManageState {
    let current = cursor_of(state);
    let mut s = state.clone();
    let (code, steps) = if target.index() >= current.index() {
        (KeyCode::Down, target.index() - current.index())
    } else {
        (KeyCode::Up, current.index() - target.index())
    };
    for _ in 0..steps {
        s = step(&s, key(code), Some(&web01())).state;
    }
    assert_eq!(
        cursor_of(&s),
        target,
        "the test helper could not move the cursor to {target:?}"
    );
    s
}

/// Press Enter on the `▶` row, wherever the cursor currently is.
fn submit(state: &ManageState) -> manage::Step {
    let at_submit = goto(state, FormCursor::Submit);
    step(&at_submit, key(KeyCode::Enter), Some(&web01()))
}

/// The form cursor, whichever of the two maps is open.
fn cursor_of(state: &ManageState) -> FormCursor {
    match &state.phase {
        Phase::Add(sequence) => sequence.cursor,
        Phase::Edit(editor) => editor.cursor,
        other => panic!("expected a form map, got {other:?}"),
    }
}

/// The live add sequence inside a state.
fn add_seq(state: &ManageState) -> &AddSequence {
    match &state.phase {
        Phase::Add(sequence) => sequence,
        other => panic!("expected the add map, got {other:?}"),
    }
}

/// The live edit sequence inside a state.
fn edit_seq(state: &ManageState) -> &EditSequence {
    match &state.phase {
        Phase::Edit(editor) => editor,
        other => panic!("expected the edit map, got {other:?}"),
    }
}

/// The last refusal shown on either map.
fn error_of(state: &ManageState) -> Option<&str> {
    match &state.phase {
        Phase::Add(sequence) => sequence.error.as_deref(),
        Phase::Edit(editor) => editor.error.as_deref(),
        other => panic!("expected a form map, got {other:?}"),
    }
}

/// Every problem with the open map, in map order.
fn problems_of(state: &ManageState) -> Vec<(&'static str, String)> {
    match &state.phase {
        Phase::Add(sequence) => sequence.problems(),
        Phase::Edit(editor) => editor.problems(),
        other => panic!("expected a form map, got {other:?}"),
    }
}

/// The glyph the map is showing for one field.
fn glyph_of(state: &ManageState, field: AddField) -> RowGlyph {
    let rows = match &state.phase {
        Phase::Add(sequence) => sequence.rows(),
        Phase::Edit(editor) => editor.rows(),
        other => panic!("expected a form map, got {other:?}"),
    };
    row(&rows, field).glyph
}

/// Find one field's row.
fn row<'a>(rows: &'a [MapRow], field: AddField) -> &'a MapRow {
    rows.iter()
        .find(|r| r.field == field)
        .unwrap_or_else(|| panic!("no row for {field:?} in {rows:?}"))
}

/// Unwrap the single `Effect::Add` a submit was expected to produce.
///
/// A helper rather than a `let-else` at each call site because the
/// assertion is always the same one — *exactly* one add, nothing else in
/// the list — and a test that unpacked `effects[0]` alone would let a
/// second, unexpected effect slip past.
fn only_add(step: &manage::Step) -> &ConnectionDraft {
    match &step.effects[..] {
        [Effect::Add { draft }] => draft,
        other => panic!("expected exactly one Add, got {other:?}"),
    }
}

/// Unwrap the single `Effect::Update` a save was expected to produce.
fn only_update(step: &manage::Step) -> (&Connection, &ConnectionDraft) {
    match &step.effects[..] {
        [Effect::Update { target, draft }] => (target, draft),
        other => panic!("expected exactly one Update, got {other:?}"),
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
