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
    /// The `Ctrl+E` in-place single-field edit (#37).
    ///
    /// The other half of the ticket, and deliberately **not** a second
    /// [`Add`]: the add sequence walks five steps because there is nothing
    /// on screen to edit, while an edit starts from a Connection that is
    /// already whole and changes exactly one field of it. User story 26
    /// asks for "one step", and a re-sequence would be five.
    ///
    /// `target` is captured by value at the chord, exactly as
    /// [`Phase::ConfirmDelete`] captures its target. Everything the editor
    /// then does is about *this* Connection; a cursor that drifts
    /// afterwards changes nothing about it.
    ///
    /// [`Add`]: Phase::Add
    Edit(EditSequence),
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

    /// The value this field holds in a draft, as a borrow.
    ///
    /// The one place that names which draft member a field *is*. Both
    /// [`Self::settled_value`] and the edit path's pre-fill read through
    /// it, so the two can never disagree about what `Port` means.
    fn field_of(self, draft: &ConnectionDraft) -> &str {
        match self {
            AddField::Alias => &draft.alias,
            AddField::Host => &draft.host,
            AddField::Port => &draft.port,
            AddField::Key => &draft.key_path,
            AddField::Folder => &draft.folder,
        }
    }

    /// The value this field holds in a live Connection, as text.
    ///
    /// What the in-place editor pre-fills the line with. Routed through
    /// [`ConnectionDraft::from_connection`] rather than reading
    /// `Connection` directly so the editor shows the same spelling the
    /// store would write — the port as digits, an absent optional as
    /// empty — and not a second formatting of its own.
    pub fn current(self, conn: &Connection) -> String {
        self.field_of(&ConnectionDraft::from_connection(conn))
            .to_string()
    }

    /// The value this field settled to, read back out of the draft.
    ///
    /// `None` is the honest shape of an optional field the user left
    /// alone: absent, not empty.
    fn settled_value(self, draft: &ConnectionDraft) -> Option<String> {
        let value = self.field_of(draft);
        (!value.is_empty()).then(|| value.to_string())
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

/// The in-place single-field editor as a value (#37).
///
/// The whole editor is one value: which Connection is being edited, which
/// of its fields is on the line, what is being typed, and what the last
/// rejection said. As with [`AddSequence`], that is what turns "what does
/// Enter do here?" into a unit test with no terminal.
///
/// **Why one field and not five.** The add sequence has to walk the fields
/// because it is building a Connection out of nothing. An edit starts
/// from a Connection that is already whole, and story 26 asks for a small
/// correction to take *one* step. Re-running the five-step sequence to
/// change a port would be four steps of nothing.
///
/// **Why the field is chosen rather than assumed.** The editor could have
/// opened on Alias every time, but then correcting a port would mean
/// arrowing past four fields with no way to see which is live. So the
/// field is part of the state, `←`/`→` move it, and the header names it —
/// the same reason the add sequence puts the field word on the header
/// instead of leaving the user to count steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSequence {
    /// The Connection being edited, captured by value at the chord.
    ///
    /// The editor changes one field *of this Connection*. Carrying the
    /// whole value is what lets the frame show the target by name, lets
    /// each field be pre-filled from what it actually holds, and lets the
    /// driver hand the store the id of the thing the user was looking at
    /// rather than whatever the cursor has since drifted onto.
    pub target: Connection,
    /// The field on the line right now.
    pub field: AddField,
    /// What is on the line. Seeded with the field's current value, so the
    /// user is editing what is there rather than retyping it.
    pub input: String,
    /// The last rejection, shown under the header and cleared by the next
    /// keystroke that could fix it.
    pub error: Option<String>,
}

impl EditSequence {
    /// Open the editor on a Connection, at its first field.
    pub fn start(target: Connection) -> Self {
        Self {
            field: AddField::Alias,
            input: AddField::Alias.current(&target),
            target,
            error: None,
        }
    }

    /// The same editor with a different field on the line, its input
    /// re-seeded from what that field actually holds.
    ///
    /// The re-seed is the whole reason moving between fields is safe: the
    /// line always shows the truth about the field it names, so a user who
    /// arrows from Host to Port and hits Enter cannot accidentally write
    /// a hostname into the port.
    ///
    /// A field with nothing settled yet comes back empty, which is the
    /// honest starting point for an optional field that has never had a
    /// value.
    fn focused(&self, field: AddField) -> Self {
        Self {
            field,
            input: field.current(&self.target),
            error: None,
            ..self.clone()
        }
    }

    /// The field `delta` steps away from the live one, wrapping at both
    /// ends.
    ///
    /// Wrapping is what makes `←` from Alias land on Folder rather than
    /// doing nothing: on a five-item cycle a key that silently dies reads
    /// as a broken keybinding, and the rail has already promised the key
    /// moves the field.
    fn shifted(&self, delta: isize) -> AddField {
        let len = AddField::ORDER.len() as isize;
        let now = AddField::ORDER
            .iter()
            .position(|f| *f == self.field)
            .unwrap_or(0) as isize;
        AddField::ORDER[((now + delta).rem_euclid(len)) as usize]
    }

    /// The Connection the draft in `draft` would replace `target` with.
    ///
    /// Carries `target`'s id across rather than minting a new one: an
    /// edit answers about *this* Connection, and a fresh id would orphan
    /// every reference to the old one. The minting half of
    /// [`connection_from`] is deliberately not used here — the store owns
    /// that, and this only owns the "same Connection, one field changed"
    /// half.
    fn edited(&self, value: String) -> ConnectionDraft {
        let mut draft = ConnectionDraft::from_connection(&self.target);
        self.field.commit(&mut draft, value);
        draft
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
    /// `Ctrl+E` changed a field **and the store really wrote it** (#37).
    ///
    /// Earned through [`settle_edit`], never by the Enter that finished
    /// the edit — the same discipline [`Trace::Added`] and
    /// [`Trace::Deleted`] are held to. `◇ edited [prod] web-01` is a
    /// claim about `connections.json`.
    ///
    /// The note names the Connection rather than the field, because the
    /// field is what the user just typed and already knows; the thing they
    /// want confirmed is *which Connection* the file changed.
    Edited { connection: Connection },
    /// The editor was abandoned. Nothing was written.
    ///
    /// The edit half of [`Trace::AddAbandoned`]: backing out of a
    /// half-typed field leaves the Connection exactly as it was, and the
    /// note answers the one question the user has rather than leaving the
    /// list to imply it.
    EditAbandoned,
    /// `Ctrl+E` was answered and the store had no such Connection: the
    /// edit changed nothing and wrote nothing.
    ///
    /// The edit half of [`Trace::DeleteFailed`]. The user pressed Enter
    /// meaning to change a Connection that was on screen when the chord
    /// was pressed; if the store no longer has it, saying `edited` would
    /// describe a write that did not happen.
    EditFailed { connection: Connection },
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
    /// Replace an existing Connection with the one the edit produced, and
    /// persist it (#37).
    ///
    /// The whole Connection travels with the request, not just the field
    /// that changed. Two reasons, both about honesty:
    ///
    /// * the frame's answer afterwards is about *this* Connection, and if
    ///   the store reports it never had one the id alone leaves the
    ///   driver nothing to name;
    /// * the store's `update` takes a draft and keeps the id, so sending
    ///   the unchanged fields as they were read means the write cannot
    ///   clobber a field the editor never touched.
    Update {
        /// The Connection the chord was pressed over.
        target: Connection,
        /// The whole Connection as it should now stand, carrying
        /// `target`'s id.
        draft: ConnectionDraft,
    },
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
    _state: &ManageState,
    connections: &[Connection],
    outcome: Result<Connection, String>,
) -> Result<ManageState, String> {
    match outcome {
        Ok(connection) => {
            // The spec's "the list is refreshed and shows the resulting
            // Connection" is a promise about the glass, not just the file.
            // Two things would break it: a filter the user typed before
            // `Ctrl+A` that the new Connection does not match (the list
            // would read `No matches` under a note naming a Connection it
            // is not showing), and a new row appended past the bottom of
            // the window. So the write clears the filter and puts the
            // cursor on the row it just made — the frame's window slides
            // to the selection, which is what brings a far row into
            // view. One rule, always shows the result.
            let selection = index_of_id(connections, &connection.id);
            Ok(ManageState {
                phase: Phase::List,
                query: String::new(),
                selection,
                trace: Some(Trace::Added { connection }),
            })
        }
        Err(message) => Err(message),
    }
}

/// The row a Connection sits on in `connections`, or 0 if it is not
/// there. The settle paths use this to land the cursor on a row they just
/// wrote; 0 is the safe fallback for a store that handed back something
/// it is not listing, which the frame clamps anyway.
fn index_of_id(connections: &[Connection], id: &str) -> usize {
    connections.iter().position(|c| c.id == id).unwrap_or(0)
}

/// What a [`Store::update`] call actually did (#37).
///
/// The edit half of [`DeleteOutcome`]. The state machine owns no disk and
/// must not guess whether the Connection it was editing is still there,
/// so the driver runs the effect and hands the answer back through
/// [`settle_edit`].
///
/// [`Store::update`]: crate::connections::Store::update
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOutcome {
    /// A Connection really was changed, and the store hands back the
    /// changed one.
    Updated(Connection),
    /// No Connection had that id: nothing was changed and nothing was
    /// written.
    Absent,
    /// The store refused the edit and said why.
    Failed(String),
}

impl EditOutcome {
    /// Classify what [`Store::update`] returned.
    ///
    /// `Ok(None)` is its own arm and not a success: the contract is that
    /// nothing was written, and a frame that reads it as "edited" is
    /// describing a file it never touched.
    ///
    /// [`Store::update`]: crate::connections::Store::update
    pub fn from_update(result: Result<Option<Connection>, String>) -> Self {
        match result {
            Ok(Some(conn)) => EditOutcome::Updated(conn),
            Ok(None) => EditOutcome::Absent,
            Err(message) => EditOutcome::Failed(message),
        }
    }
}

/// Fold the store's answer to [`Effect::Update`] into the state the frame
/// is rebuilt from (#37).
///
/// The edit half of the honesty rule, mirroring [`settle_add`] and
/// [`settle_delete`]:
///
/// * `Updated(conn)` — the note earns `edited`, and names the Connection
///   the store actually wrote, not the one the editor started from. If
///   the store changed something on the way in, the frame reports what
///   came back.
/// * `Absent` — the user said yes to editing a Connection the store does
///   not have. The frame stays up and says the edit did not happen,
///   because a `◇ edited` over a list that still shows the old row would
///   be a flat contradiction of the rows underneath it.
/// * `Failed(message)` — the caller collapses the frame to `◆ error …`,
///   the same way a refused add and a refused delete do.
pub fn settle_edit(
    state: &ManageState,
    connections: &[Connection],
    target: &Connection,
    outcome: EditOutcome,
) -> Result<ManageState, String> {
    match outcome {
        EditOutcome::Updated(connection) => {
            // Same promise as the add: the refreshed list must show the
            // Connection that changed. The edit can move a row out from
            // under the filter (rename `web-01` to `db-01` while the
            // filter reads `web`), so the write clears the filter and
            // lands the cursor on the changed row.
            let selection = index_of_id(connections, &connection.id);
            Ok(ManageState {
                phase: Phase::List,
                query: String::new(),
                selection,
                trace: Some(Trace::Edited { connection }),
            })
        }
        EditOutcome::Absent => Ok(ManageState {
            trace: Some(Trace::EditFailed {
                connection: target.clone(),
            }),
            ..state.clone()
        }),
        EditOutcome::Failed(message) => Err(message),
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
        Phase::Edit(ref editor) => edit_step(state, key, editor),
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
        // The in-place single-field editor (#37). Like `Ctrl+X`, the
        // target is captured *here*, by value: everything the editor
        // then does — including arrowing across five fields — is about
        // the Connection the user was pointing at when they pressed the
        // chord. With nothing under the cursor it asks for nothing,
        // because the frame will not invent a target to change.
        if let Some(target) = selected {
            return Step::state_only(ManageState {
                phase: Phase::Edit(EditSequence::start(target.clone())),
                // The note from the last action is cleared the same way
                // `Ctrl+A` clears it: the editor is a new ask, and a
                // stale `◇ deleted` sitting above "which field am I
                // editing?" reads as an answer to the wrong question.
                trace: None,
                ..state.clone()
            });
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
        Some(field) => {
            // The line is re-seeded from the field being landed on, the
            // same rule [`AddSequence::back`] follows. On the first pass
            // through the draft's next field is empty, so this is the
            // blank line it always was; on a pass that has already been
            // back through, it is the answer the field holds. Either way
            // the line shows the truth about the field it names — which
            // is what stops a bare Enter on a re-entered field from
            // re-settling it from nothing and silently wiping the answer
            // given the first time.
            let input = field.settled_value(&draft).unwrap_or_default();
            Step::state_only(ManageState {
                phase: Phase::Add(AddSequence {
                    field,
                    draft,
                    input,
                    error: None,
                }),
                ..state.clone()
            })
        }
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

/// The in-place single-field editor (#37).
///
/// The same three rules the add sequence is built on, with one added:
///
/// * **Typing edits the field, never the filter behind it.** The filter
///   is what the user returns to.
/// * **A rejection is not a transition.** A bad port leaves the field,
///   the typed text and the target exactly where they were, plus one line
///   saying why.
/// * **Nothing is written until Enter**, and even then the editor only
///   *asks* — [`settle_edit`] is where the frame learns whether the
///   write happened.
/// * **Movement changes which field is on the line, never which
///   Connection is being edited.** The target was captured at the chord.
///   Arrowing left and right across five fields cannot move the edit onto
///   a neighbour, for the same reason a cursor cannot move a delete onto
///   one: the user answered about the thing they were looking at.
fn edit_step(state: &ManageState, key: KeyEvent, editor: &EditSequence) -> Step {
    if is_chord(key, 'c') {
        return Step::exit(state.clone(), InlineOutcome::Cancelled);
    }

    // Every arm below produces the editor state the frame becomes;
    // `editing` wraps it back up so no arm re-types the spread.
    let editing = |editor: EditSequence| {
        Step::state_only(ManageState {
            phase: Phase::Edit(editor),
            ..state.clone()
        })
    };

    match key.code {
        KeyCode::Enter => commit_field(state, editor),
        // Backing out of an edit is not back *one step* — there is only
        // one step. It is backing out of the whole thing, and what the
        // user needs answered is whether the half-typed field got saved.
        // It did not: no `Effect::Update` leaves this module until Enter
        // validates it.
        KeyCode::Esc => Step::state_only(ManageState {
            phase: Phase::List,
            trace: Some(Trace::EditAbandoned),
            ..state.clone()
        }),
        KeyCode::Left | KeyCode::BackTab => editing(editor.focused(editor.shifted(-1))),
        KeyCode::Right | KeyCode::Tab => editing(editor.focused(editor.shifted(1))),
        KeyCode::Backspace => {
            let mut next = editor.clone();
            next.input.pop();
            // The user is fixing it. Keeping the rejection up after the
            // first keystroke of the fix would be reporting a mistake
            // that is already on its way out.
            next.error = None;
            editing(next)
        }
        KeyCode::Char(ch) if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() => {
            let mut next = editor.clone();
            next.input.push(ch);
            next.error = None;
            editing(next)
        }
        _ => Step::state_only(state.clone()),
    }
}

/// Answer the live field: validate, then ask the store to write — or
/// refuse.
///
/// The refusal keeps the field, the input and the target and adds only the
/// reason. The acceptance is the one keystroke in the editor that asks for
/// a write, and it does not claim one: `trace` stays empty until
/// [`settle_edit`] folds the store's answer in.
fn commit_field(state: &ManageState, editor: &EditSequence) -> Step {
    let value = match editor.field.settle(&editor.input) {
        Ok(value) => value,
        Err(message) => {
            return Step::state_only(ManageState {
                phase: Phase::Edit(EditSequence {
                    error: Some(message),
                    ..editor.clone()
                }),
                ..state.clone()
            })
        }
    };

    Step {
        state: ManageState {
            phase: Phase::List,
            trace: None,
            ..state.clone()
        },
        effects: vec![Effect::Update {
            target: editor.target.clone(),
            draft: editor.edited(value),
        }],
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
