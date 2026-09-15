//! The manage frame's interaction, as a pure state machine (#36).
//!
//! [`crate::frame`] turns a state into a `Frame`: a value. [`crate::inline`]
//! puts that value on the glass. This module is the thing in between that
//! neither of them can own and no terminal can prove: **which keystroke means
//! what**.
//!
//! That decision used to be smeared across the key loop in `run_inline`, where
//! testing it meant driving a PTY — and a PTY cannot answer the question that
//! actually matters about a delete: *was the Connection that was on screen
//! when the chord was pressed the one that got removed?* Here the whole
//! interaction is a function of `(state, key, selection)`, so every dangerous
//! path is a unit test with no terminal in sight.
//!
//! Three rules bind the module:
//!
//! 1. **No I/O.** No file, no terminal, no clock, no randomness. The state
//!    machine decides; the driver performs. Effects leave as data
//!    ([`Effect`]) and the caller runs them through
//!    [`crate::connections::Store`].
//! 2. **A printable is never a management action.** Every printable character
//!    the user can hit goes to the filter. The management chords are Ctrl
//!    chords precisely so that `a`, `e` and `x` can be typed into a search
//!    for what they spell (user story 21).
//! 3. **The confirm is scoped to the target, not to the cursor.** `Ctrl+X`
//!    captures the Connection under the cursor *at that moment* into
//!    [`Phase::ConfirmDelete`]. Nothing the user types afterwards — including
//!    movement — can move that target onto a different Connection. The delete
//!    answers about the captured one or not at all.
//!
//! The frame's constant height and the settle-collapse are #34's and are
//! untouched: this module only says which lines the frame should be showing.

use crate::config::Connection;
use crate::inline::InlineOutcome;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The step the manage frame is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// The list: the filter is live and the cursor picks a target.
    List,
    /// The inline delete confirm.
    ///
    /// `target` is the Connection the cursor was on when `Ctrl+X` was
    /// pressed, captured by value. The confirm asks about *this* one; a
    /// cursor that drifts afterwards changes nothing about it.
    ConfirmDelete { target: Connection },
}

/// The trace the last management action leaves on the list.
///
/// Rendered by the frame as the settled `■` line plus the dim `◇` note
/// (#36). It is kept on the state rather than being a one-frame flash so the
/// user can read it after the confirm collapses, and so a second look at the
/// frame answers "what just happened" without scrolling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trace {
    /// The confirm was answered `y`: this is the Connection that was removed.
    Deleted { connection: Connection },
    /// The confirm was answered no, or abandoned. Nothing changed.
    Declined { connection: Connection },
    /// `Ctrl+A` was routed to the add path.
    ///
    /// The interactive add step-sequence is #37's work. Until it lands the
    /// route ends here, with the frame answering the chord with a dim note
    /// rather than swallowing it — a chord that does nothing is worse than
    /// one that says what is missing.
    AddRequested,
}

/// Everything the manage frame's interaction consists of.
///
/// The frame is a function of this value, and this value is a function of the
/// keys, which is what makes the whole surface reviewable as data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManageState {
    /// The live filter text.
    pub query: String,
    /// The row the cursor is on, in the frame's own (filtered) coordinates.
    ///
    /// The frame clamps it to the rows that exist and the driver writes the
    /// clamped value back, exactly as the pick loop does.
    pub selection: usize,
    /// Which step the frame is on.
    pub phase: Phase,
    /// What the last management action did, shown above the rows.
    pub trace: Option<Trace>,
}

impl Default for ManageState {
    fn default() -> Self {
        Self::new()
    }
}

impl ManageState {
    /// An empty list step.
    pub fn new() -> Self {
        Self {
            query: String::new(),
            selection: 0,
            phase: Phase::List,
            trace: None,
        }
    }

    /// A list step seeded with a filter.
    pub fn with_query(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            ..Self::new()
        }
    }
}

/// What the driver must go and do because of a key.
///
/// Data, not a callback: the state machine says what should happen and the
/// caller decides how, which is what keeps this module free of the terminal
/// and of the disk while still driving both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Persist the removal of the Connection with this id.
    ///
    /// Emitted only from the confirm step, and only for the captured target.
    Delete { id: String },
    /// Start adding a Connection.
    ///
    /// #37 replaces what the driver does with this — the `◆ Alias` →
    /// `◆ Host` step-sequence. It is a real route today: the chord reaches
    /// the driver as a request, and the driver answers it with a note.
    BeginAdd,
    /// Leave the frame with this outcome.
    Exit(InlineOutcome),
}

/// The outcome of one keystroke: the state to become, and the effects to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub state: ManageState,
    pub effects: Vec<Effect>,
}

impl Step {
    /// A keystroke that changed the state and asks for nothing.
    fn state_only(state: ManageState) -> Self {
        Self {
            state,
            effects: Vec::new(),
        }
    }

    /// A keystroke that leaves the frame.
    fn exit(state: ManageState, outcome: InlineOutcome) -> Self {
        Self {
            state,
            effects: vec![Effect::Exit(outcome)],
        }
    }
}

/// Advance the manage interaction by one key.
///
/// `selected` is the Connection the frame's cursor is pointing at, or `None`
/// when the frame has no row to point at (empty list, no match). A chord
/// with nothing selected asks for nothing: the frame does not guess a target.
pub fn step(state: &ManageState, key: KeyEvent, selected: Option<&Connection>) -> Step {
    match state.phase {
        Phase::List => list_step(state, key, selected),
        Phase::ConfirmDelete { ref target } => confirm_step(state, key, target),
    }
}

/// The list step: filter, movement, and the chords that start an action.
///
/// This is the whole of the manage frame's key behaviour while it is just a
/// list, so that the driver has one place to ask rather than a `match` that
/// has to remember which of these the frame is allowed to act on.
fn list_step(state: &ManageState, key: KeyEvent, selected: Option<&Connection>) -> Step {
    if is_chord(key, 'c') {
        return Step::exit(state.clone(), InlineOutcome::Cancelled);
    }

    if is_chord(key, 'x') {
        // The target is captured *here*, by value. Everything the confirm then
        // does — and refuses to do — is about this Connection, whatever the
        // cursor gets up to between now and the answer.
        if let Some(target) = selected {
            return Step::state_only(ManageState {
                phase: Phase::ConfirmDelete {
                    target: target.clone(),
                },
                ..state.clone()
            });
        }
        return Step::state_only(state.clone());
    }

    if is_chord(key, 'e') {
        // Enter's meaning under `sshm manage`, on a chord: the selection
        // leaves the frame on the edit route (#35's contract). #37 turns
        // the destination into the in-place single-field editor.
        if let Some(conn) = selected {
            return Step::exit(state.clone(), InlineOutcome::Picked(conn.clone()));
        }
        return Step::state_only(state.clone());
    }

    if is_chord(key, 'a') {
        return Step {
            state: ManageState {
                trace: Some(Trace::AddRequested),
                ..state.clone()
            },
            effects: vec![Effect::BeginAdd],
        };
    }

    let mut next = state.clone();
    match key.code {
        KeyCode::Esc => return Step::exit(next, InlineOutcome::Cancelled),
        KeyCode::Enter => {
            if let Some(conn) = selected {
                return Step::exit(next, InlineOutcome::Picked(conn.clone()));
            }
        }
        KeyCode::Up => next.selection = next.selection.saturating_sub(1),
        KeyCode::Down => next.selection = next.selection.saturating_add(1),
        KeyCode::Backspace => {
            next.query.pop();
        }
        // `hjkl` is the frame's existing movement binding, carried over from
        // the pick loop unchanged. It is not a management action, so it does
        // not collide with rule 2 — but it does mean `j` and `k` are not
        // filter text, which the sweep test names explicitly.
        KeyCode::Char('j') if key.modifiers.is_empty() => next.selection += 1,
        KeyCode::Char('k') if key.modifiers.is_empty() => {
            next.selection = next.selection.saturating_sub(1)
        }
        KeyCode::Char(ch) if key.modifiers.is_empty() => next.query.push(ch),
        _ => return Step::state_only(state.clone()),
    }

    Step::state_only(next)
}

/// The confirm step: `y` deletes, everything else declines.
///
/// The answer set is deliberately tiny and the default is No:
///
/// * `y` / `Y` — confirm. One delete, of the captured target.
/// * `n` / `N`, `Esc` — decline. Nothing is asked for.
/// * any other printable — decline **and** filter. The filter is live right
///   through the confirm (user story 21), so a user who hits `Ctrl+X` by
///   accident and carries on typing ends up searching, not deleting. The
///   character is not wasted and is not an answer.
/// * anything else — ignored. Movement in particular does nothing here: the
///   target was captured when the chord was pressed, so there is nothing for
///   the cursor to change.
fn confirm_step(state: &ManageState, key: KeyEvent, target: &Connection) -> Step {
    if is_chord(key, 'c') {
        return Step::exit(state.clone(), InlineOutcome::Cancelled);
    }

    if is_answer(key, 'y') {
        return Step {
            state: ManageState {
                phase: Phase::List,
                trace: Some(Trace::Deleted {
                    connection: target.clone(),
                }),
                ..state.clone()
            },
            effects: vec![Effect::Delete {
                id: target.id.clone(),
            }],
        };
    }

    if is_answer(key, 'n') || key.code == KeyCode::Esc {
        return Step::state_only(declined(state, target));
    }

    if let KeyCode::Char(ch) = key.code {
        if key.modifiers.is_empty() {
            let mut next = declined(state, target);
            next.query.push(ch);
            return Step::state_only(next);
        }
    }

    Step::state_only(state.clone())
}

/// The state after a confirm that did not delete: back on the list, with the
/// decline left as the trace.
fn declined(state: &ManageState, target: &Connection) -> ManageState {
    ManageState {
        phase: Phase::List,
        trace: Some(Trace::Declined {
            connection: target.clone(),
        }),
        ..state.clone()
    }
}

/// Is this key the unadorned answer `ch` to the `(y/N)` prompt?
///
/// Shift is allowed (`Y` answers the same as `y`); every other modifier is
/// not, because a Ctrl chord is a *different question* and must not be read
/// as an answer to this one.
fn is_answer(key: KeyEvent, ch: char) -> bool {
    key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
        && matches!(key.code, KeyCode::Char(c) if c.eq_ignore_ascii_case(&ch))
}

/// Is this key the Ctrl chord `ch`?
///
/// Matched case-insensitively and with `CONTROL` required but not exclusive:
/// a terminal may report `Ctrl+Shift+A` as `Char('A')` with both modifiers,
/// and a chord the user pressed should not be dropped over the shift. What it
/// may **not** be is a bare printable — that is the whole basis of rule 2.
fn is_chord(key: KeyEvent, ch: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(c) if c.eq_ignore_ascii_case(&ch))
}
