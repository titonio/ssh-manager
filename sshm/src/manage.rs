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
use crate::connections::ConnectionDraft;
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
    /// The `Ctrl+A` add step-sequence (#37).
    ///
    /// The whole sequence is one value: which field is live, what has been
    /// settled so far, what is being typed right now, and what the last
    /// rejection said. That is what makes "what does Enter do on the Port
    /// step" a question a unit test can answer with no terminal.
    Add(AddSequence),
}

/// Which field of the add sequence is live.
///
/// The five are the spec's five, in the spec's order (#31 user story 24).
/// Each one carries its own optionality and its own validation, and both
/// come from the `Connection` model rather than from taste:
///
/// * **Alias, Host** — required. A `Connection` without one of these is
///   not a Connection: no alias and there is nothing to pick or emit; no
///   host and `ssh` has nowhere to go. Empty is rejected.
/// * **Port** — optional with the SSH default. Empty settles as `22`,
///   which is what [`ConnectionDraft::clear`] seeds and what
///   `connections::connection_from` falls back to. Anything that is not
///   a number in `1..=65535` is rejected, because a `port` that silently
///   becomes `22` after the user typed `99999` is a Connection that
///   connects to the wrong machine.
/// * **Key, Folder** — optional, and *absent* when left empty.
///   `Connection::key_path` and `::folder` are `Option<String>`, and a
///   settled empty string would reach `ssh` as `-i ""`.
///
/// There is deliberately **no User step.** The spec names five steps and
/// `user` is not one of them, so the sequence cannot collect one and the
/// added Connection carries an empty `user` — exactly what `sshm add`
/// without `--user` produces, and what `ssh::build_ssh_args` already
/// treats as "no `user@`, let ssh use the local login name". Adding the
/// field is a spec change, not this ticket's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddField {
    Alias,
    Host,
    Port,
    Key,
    Folder,
}

impl AddField {
    /// The order the sequence walks, as the spec writes it.
    pub const ORDER: [AddField; 5] = [
        AddField::Alias,
        AddField::Host,
        AddField::Port,
        AddField::Key,
        AddField::Folder,
    ];

    /// The step's label — the word that wears the `◆` in the header and
    /// the `◇` once it has settled.
    pub fn label(self) -> &'static str {
        match self {
            AddField::Alias => "Alias",
            AddField::Host => "Host",
            AddField::Port => "Port",
            AddField::Key => "Key",
            AddField::Folder => "Folder",
        }
    }

    /// The next step, or `None` after the last one — which is the answer
    /// that turns "settle this field" into "add the Connection".
    pub fn next(self) -> Option<AddField> {
        let i = Self::ORDER.iter().position(|f| *f == self)?;
        Self::ORDER.get(i + 1).copied()
    }

    /// Whether this field must be answered with something.
    fn required(self) -> bool {
        matches!(self, AddField::Alias | AddField::Host)
    }

    /// Validate and normalise one step's answer.
    ///
    /// `Ok(value)` is what settles into the draft — trimmed, and for the
    /// port step the empty answer already replaced with the SSH default.
    /// `Err(message)` is the sentence the frame shows while the user stays
    /// on this step with what they typed still there.
    ///
    /// The empty-answer rule is asked of [`AddField::required`], not
    /// re-decided here: a required field never settles empty, and an
    /// optional one falls through to its own default — the port to the
    /// SSH default, everything else to absent. One field list stating
    /// which-is-which, in one place; a second `matches!` here would be a
    /// second answer that the two could silently disagree about.
    fn settle(self, raw: &str) -> Result<String, String> {
        let value = raw.trim();

        if value.is_empty() {
            if self.required() {
                return Err(format!("{} is required", self.label().to_lowercase()));
            }
            return match self {
                AddField::Port => Ok(DEFAULT_PORT.to_string()),
                _ => Ok(String::new()),
            };
        }

        match self {
            AddField::Port => match value.parse::<u16>() {
                Ok(0) | Err(_) => Err("port must be a number from 1 to 65535".to_string()),
                Ok(port) => Ok(port.to_string()),
            },
            _ => Ok(value.to_string()),
        }
    }

    /// The value this field settled to, read back out of the draft.
    ///
    /// `None` is the honest shape of an optional field the user left
    /// alone: absent, not empty.
    fn settled_value(self, draft: &ConnectionDraft) -> Option<String> {
        let value = match self {
            AddField::Alias => draft.alias.clone(),
            AddField::Host => draft.host.clone(),
            AddField::Port => draft.port.clone(),
            AddField::Key => draft.key_path.clone(),
            AddField::Folder => draft.folder.clone(),
        };
        (!value.is_empty()).then_some(value)
    }

    /// Write a settled value into the draft.
    fn commit(self, draft: &mut ConnectionDraft, value: String) {
        match self {
            AddField::Alias => draft.alias = value,
            AddField::Host => draft.host = value,
            AddField::Port => draft.port = value,
            AddField::Key => draft.key_path = value,
            AddField::Folder => draft.folder = value,
        }
    }
}

/// The SSH default port, spelled once.
const DEFAULT_PORT: u16 = 22;

/// The add sequence as a value: where it has got to, and what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddSequence {
    /// The field being answered right now.
    pub field: AddField,
    /// The fields already settled. Nothing here has been written anywhere:
    /// the draft becomes a Connection only when the last step is answered,
    /// and even then it is the store that decides whether it exists.
    pub draft: ConnectionDraft,
    /// What the user has typed into the live field.
    pub input: String,
    /// The last rejection, shown under the header. Cleared by the next
    /// keystroke, so a fixed mistake stops being reported.
    pub error: Option<String>,
}

impl AddSequence {
    /// A sequence at its first step.
    pub fn start() -> Self {
        Self {
            field: AddField::Alias,
            draft: ConnectionDraft::default(),
            input: String::new(),
            error: None,
        }
    }

    /// The steps already settled, as `(label, value)`, in order.
    ///
    /// Everything before the live field. The frame draws these as the
    /// `◇` trace lines the spec asks for, and an absent optional field
    /// is `None` so the frame can say *absent* rather than show a blank.
    pub fn settled(&self) -> Vec<(&'static str, Option<String>)> {
        AddField::ORDER
            .iter()
            .take_while(|f| **f != self.field)
            .map(|f| (f.label(), f.settled_value(&self.draft)))
            .collect()
    }

    /// The step before this one, with its answer put back on the line for
    /// editing. `None` at the first step, which is where the sequence is
    /// abandoned rather than walked back.
    fn back(&self) -> Option<Self> {
        let i = AddField::ORDER.iter().position(|f| *f == self.field)?;
        if i == 0 {
            return None;
        }
        let field = AddField::ORDER[i - 1];
        Some(Self {
            field,
            draft: self.draft.clone(),
            input: field.settled_value(&self.draft).unwrap_or_default(),
            error: None,
        })
    }
}

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
    /// `Ctrl+A` ran to the end **and the store really wrote the
    /// Connection**.
    ///
    /// Earned through [`settle_add`], never by the keystroke that
    /// finished the sequence — the same discipline [`Trace::Deleted`]
    /// is held to. `◇ added [prod] web-01` is a claim about
    /// `connections.json`, and only the store gets to make it.
    Added { connection: Connection },
    /// The sequence was walked back off its first step. Nothing was
    /// written, and the note says so rather than leaving the user to
    /// wonder whether a half-filled Connection got saved.
    AddAbandoned,
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
    /// Build a Connection from the completed draft and persist it.
    ///
    /// The whole draft travels with the request because the frame's answer
    /// afterwards is about the Connection the draft became, and only the
    /// store knows whether it became one. The state machine does not
    /// build a `Connection` here: minting an id is the store's job, and
    /// so is the write.
    Add { draft: ConnectionDraft },
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

/// Fold the store's answer to [`Effect::Add`] into the state the frame is
/// rebuilt from.
///
/// The add half of the honesty rule, mirroring [`settle_delete`]: the
/// sequence finishing is what the user *did*, and `◇ added` is a claim
/// about the file. A draft that came back `Err` never earns the word —
/// the Connection is not in `connections.json`, and a frame that said it
/// was would be describing a write that did not happen.
///
/// * `Ok(conn)` — the note earns `added`, and names the Connection the
///   store actually built (its id, its folder, its port — not what the
///   user typed at step one).
/// * `Err(message)` — the store refused. The caller collapses the frame
///   to `◆ error …`, the same way a refused delete does: a live list the
///   store cannot vouch for is worse than no list.
pub fn settle_add(
    state: &ManageState,
    outcome: Result<Connection, String>,
) -> Result<ManageState, String> {
    match outcome {
        Ok(connection) => Ok(ManageState {
            phase: Phase::List,
            trace: Some(Trace::Added { connection }),
            ..state.clone()
        }),
        Err(message) => Err(message),
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
        Phase::Add(ref sequence) => add_step(state, key, sequence),
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
        // The sequence opens on the list's own body: the filter and the
        // cursor come back with the user when it ends. What it replaces is
        // the note, never the list.
        return Step::state_only(ManageState {
            phase: Phase::Add(AddSequence::start()),
            trace: None,
            ..state.clone()
        });
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

/// The add step-sequence: one field on the line at a time.
///
/// Three rules the whole sequence is built on:
///
/// * **Typing edits the live field, never the filter behind it.** The
///   filter is what the user returns to; the sequence borrows the frame,
///   not their search.
/// * **A rejection is not a transition.** An answer that fails
///   validation leaves the step, the input and the cursor exactly where
///   they were and adds one line saying why. The user fixes the field;
///   they do not start the step over.
/// * **Nothing is written until the last step is answered**, and even
///   then the sequence only *asks* — [`settle_add`] is where the frame
///   learns whether the write happened.
fn add_step(state: &ManageState, key: KeyEvent, sequence: &AddSequence) -> Step {
    if is_chord(key, 'c') {
        return Step::exit(state.clone(), InlineOutcome::Cancelled);
    }

    let mut next = sequence.clone();
    match key.code {
        KeyCode::Enter => return settle_field(state, sequence),
        KeyCode::Esc => {
            // Back one step, with that step's answer back on the line to
            // be corrected. Backing off the *first* step is backing out of
            // the sequence: there is nothing before it, and what the user
            // needs answered is whether the half of it they typed got
            // saved. It did not — no `Effect::Add` leaves this module
            // until the last step is answered — and the note says so.
            return match sequence.back() {
                Some(previous) => Step::state_only(ManageState {
                    phase: Phase::Add(previous),
                    ..state.clone()
                }),
                None => Step::state_only(ManageState {
                    phase: Phase::List,
                    trace: Some(Trace::AddAbandoned),
                    ..state.clone()
                }),
            };
        }
        KeyCode::Backspace => {
            next.input.pop();
            next.error = None;
        }
        KeyCode::Char(ch) if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() => {
            next.input.push(ch);
            // The user is fixing it. Keep saying it is broken after the
            // first keystroke of the fix would be reporting a mistake
            // that is already on its way out.
            next.error = None;
        }
        _ => return Step::state_only(state.clone()),
    }

    Step::state_only(ManageState {
        phase: Phase::Add(next),
        ..state.clone()
    })
}

/// Answer the live step: validate, settle, advance — or refuse.
///
/// A refusal keeps everything: the step, the input, the frame. The only
/// thing it adds is the reason. An acceptance writes the value into the
/// draft and moves on, and the last step's acceptance is the one thing in
/// the whole sequence that asks the driver for anything.
fn settle_field(state: &ManageState, sequence: &AddSequence) -> Step {
    let value = match sequence.field.settle(&sequence.input) {
        Ok(value) => value,
        Err(message) => {
            return Step::state_only(ManageState {
                phase: Phase::Add(AddSequence {
                    error: Some(message),
                    ..sequence.clone()
                }),
                ..state.clone()
            })
        }
    };

    let mut draft = sequence.draft.clone();
    sequence.field.commit(&mut draft, value);

    match sequence.field.next() {
        Some(field) => Step::state_only(ManageState {
            phase: Phase::Add(AddSequence {
                field,
                draft,
                input: String::new(),
                error: None,
            }),
            ..state.clone()
        }),
        // The last step. The sequence is done and the draft is complete,
        // which is the only point at which this module asks for a write —
        // and it still does not claim one. `trace` stays empty: the
        // `◇ added` note is earned from the store's answer, through
        // [`settle_add`], exactly as `◇ deleted` is.
        None => Step {
            state: ManageState {
                phase: Phase::List,
                trace: None,
                ..state.clone()
            },
            effects: vec![Effect::Add { draft }],
        },
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
