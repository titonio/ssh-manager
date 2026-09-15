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
    /// The confirm was answered `y` **and the store really removed it**.
    ///
    /// Earned through [`settle_delete`], never set by the keystroke: this is
    /// the frame telling the user their `connections.json` lost a
    /// Connection, and only the store gets to say whether it did.
    Deleted { connection: Connection },
    /// The confirm was answered `y` and the store had no such Connection:
    /// nothing was deleted and nothing was written.
    ///
    /// Distinct from [`Trace::Declined`] — the user *did* say yes — and from
    /// [`Trace::Deleted`], which this exists to forbid when the removal did
    /// not happen. The frame says so rather than letting the list go on
    /// showing a Connection it just claimed to have deleted.
    DeleteFailed { connection: Connection },
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
///
/// **The pick path drives this value too**, and only two of its four fields
/// mean anything there: `run_inline`'s pick branch reads and writes
/// `query`/`selection` and never touches `phase` or `trace`, which stay at
/// their defaults — and `build_frame_with_flow` discards the flow projection
/// for a pick frame anyway, because a pick frame asks no questions and
/// leaves no notes. A shared `InlineState` would name that split better.
/// It is not taken here because the rename is 58 sites across six files —
/// including `manage_test.rs` and `manage_frame_test.rs`, where a
/// flow-neutral type called `InlineState` reads worse than the honest
/// `ManageState` in the file that tests the manage flow — and because the
/// change would sit on top of this commit's behaviour changes and make
/// *those* harder to review. The cheap half of the fix is this note; the
/// expensive half is the pick path getting its own state, which is worth
/// doing when it next has a reason to change.
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
    /// Persist the removal of the Connection the confirm named.
    ///
    /// Emitted only from the confirm step, and only for the captured target.
    /// The whole Connection travels with the request rather than just its
    /// id, because the answer the frame owes the user afterwards is about
    /// *this* Connection — and if the store reports it never had one, the
    /// id alone leaves the driver nothing honest to say.
    Delete { target: Connection },
    /// Start adding a Connection.
    ///
    /// #37 replaces what the driver does with this — the `◆ Alias` →
    /// `◆ Host` step-sequence. It is a real route today: the chord reaches
    /// the driver as a request, and the driver answers it with a note.
    BeginAdd,
    /// Leave the frame with this outcome.
    Exit(InlineOutcome),
}

/// What a [`Store::remove`] call actually did.
///
/// The state machine cannot know this: it owns no disk, and it must not
/// guess. The driver runs the effect and hands the result back through
/// [`settle_delete`], which is the one place the frame's claim about a
/// deletion is checked against the deletion.
///
/// [`Store::remove`]: crate::connections::Store::remove
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteOutcome {
    /// A Connection really was removed, and the store hands it back.
    Removed(Connection),
    /// No Connection had that id: nothing was deleted and nothing written.
    Absent,
    /// The store refused the delete and said why.
    Failed(String),
}

impl DeleteOutcome {
    /// Classify what [`Store::remove`] returned.
    ///
    /// `Ok(None)` is its own arm and not a success: the module's contract is
    /// that nothing was written, and a frame that reads it as "deleted" is
    /// describing a file it never touched.
    ///
    /// [`Store::remove`]: crate::connections::Store::remove
    pub fn from_remove(result: Result<Option<Connection>, String>) -> Self {
        match result {
            Ok(Some(conn)) => DeleteOutcome::Removed(conn),
            Ok(None) => DeleteOutcome::Absent,
            Err(message) => DeleteOutcome::Failed(message),
        }
    }
}

/// Fold the store's answer to [`Effect::Delete`] into the state the frame is
/// rebuilt from.
///
/// This is the seam that keeps the `◇ deleted` note honest. The keystroke
/// says what the user asked for; only this function decides what the frame
/// may then claim:
///
/// * [`DeleteOutcome::Removed`] — the note earns `deleted`.
/// * [`DeleteOutcome::Absent`] — the frame stays up and says the delete did
///   not happen. The Connection is still on disk and the list still shows
///   it, so a `deleted` note here would be a flat contradiction of the
///   rows underneath it.
/// * [`DeleteOutcome::Failed`] — `Err`, and the caller collapses the frame:
///   `error` from #31's `Settled` set (story 34). A live list of
///   Connections the store cannot vouch for is worse than no list, and a
///   painted frame the user cannot read the failure in is the failure this
///   replaced.
///
/// [`Ok(None)`]: crate::connections::Store::remove
pub fn settle_delete(
    state: &ManageState,
    target: &Connection,
    outcome: DeleteOutcome,
) -> Result<ManageState, String> {
    match outcome {
        DeleteOutcome::Removed(conn) => Ok(ManageState {
            trace: Some(Trace::Deleted { connection: conn }),
            ..state.clone()
        }),
        DeleteOutcome::Absent => Ok(ManageState {
            trace: Some(Trace::DeleteFailed {
                connection: target.clone(),
            }),
            ..state.clone()
        }),
        DeleteOutcome::Failed(message) => Err(message),
    }
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
        // Every remaining printable is filter text. `j` and `k` are **not**
        // bound to movement here, deliberately: they were carried over from
        // the pick path, and they made `jakarta` untypeable — a filter with
        // two holes is not the "live filter field that accepts every
        // printable character" user story 21 asks for. The manage frame
        // moves with the arrows, which is what its hint rail names; the
        // pick frame keeps `hjkl` because story 21 is a manage requirement
        // and nothing here needs to win an argument with it.
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
        // The ask, not the claim. `trace` deliberately stays empty: whether
        // anything was deleted is a fact about the disk, and the disk has
        // not been asked yet. The driver runs the effect and folds the
        // answer back through [`settle_delete`], which is what puts
        // `deleted` — or the honest substitute — on the frame.
        return Step {
            state: ManageState {
                phase: Phase::List,
                trace: None,
                ..state.clone()
            },
            effects: vec![Effect::Delete {
                target: target.clone(),
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
