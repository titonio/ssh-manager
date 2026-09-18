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

use crate::config::{self, Connection};
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
    /// The first-run import offer (#38).
    ///
    /// `count` is what the scan found at `path`: the Host stanzas in
    /// that file that could become Connections. It is a number about a
    /// file, not a promise about the set — the offer is made before
    /// anything is written, and the number the user is *then* told
    /// arrived comes back through [`ImportOutcome`] rather than being
    /// carried forward from here.
    ///
    /// `path` is the file the user was shown, and the file the write is
    /// about. Carrying it in the phase is what stops the driver from
    /// re-deriving `~/.ssh/config` at the moment of the write and
    /// importing something the user never agreed to. The offer is also
    /// the only phase that arrives unasked: nobody pressed a chord to
    /// open it, which is why its answer set is as conservative as the
    /// delete confirm's.
    ConfirmImport { count: usize, path: String },
}

/// Which field of the form a map row stands for.
///
/// Six fields, each carrying its own optionality and its own validation,
/// and both come from the `Connection` model rather than from taste:
///
/// * **Alias, Host** — required. A `Connection` without one of these is
///   not a Connection: no alias and there is nothing to pick or emit; no
///   host and `ssh` has nowhere to go. Empty is rejected.
/// * **User** — optional, and *absent* when left empty. `Connection::user`
///   is a plain `String` that `ssh::build_ssh_args` reads as "no `user@`,
///   let ssh use the local login name", so leaving it blank is a real
///   answer, not a missing one.
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
/// **User is here because it was missing.** The original five-step sequence
/// named in #31 story 24 has no User step, so every Connection added
/// through it carried an empty `user` and rendered as `(@192.168.31.7:22)`
/// — a Connection the user could not name the login for, and could only
/// repair afterwards. That was a spec gap, not a design: `sshm add --user`
/// has always accepted one. See `docs/adr/0001-form-map-replaces-the-stepped-add.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddField {
    Alias,
    Host,
    User,
    Port,
    Key,
    Folder,
}

impl AddField {
    /// The order the map walks, top to bottom.
    pub const ORDER: [AddField; 6] = [
        AddField::Alias,
        AddField::Host,
        AddField::User,
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
            AddField::User => "User",
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
                // Short on purpose. The full sentence ("must be a number
                // from 1 to 65535") plus the example plus two other
                // problems on the line overruns 80 columns; this leaves
                // room for all three problems *and* the example.
                Ok(0) | Err(_) => Err("port must be 1–65535 (e.g. 22)".to_string()),
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
            AddField::User => &draft.user,
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

    /// Why this field is wrong, if it is. `None` means it is fine.
    ///
    /// Both kinds of wrong come through here — empty-and-required, and
    /// content that will not validate — so the map's glyph and the error
    /// line cannot disagree about which fields are wrong. They are the
    /// same question asked once.
    fn problem(self, draft: &ConnectionDraft) -> Option<String> {
        let raw = self.field_of(draft);
        if raw.trim().is_empty() {
            return self
                .required()
                .then(|| format!("{} is required", self.label().to_lowercase()));
        }
        self.settle(raw).err()
    }

    /// Write a settled value into the draft.
    fn commit(self, draft: &mut ConnectionDraft, value: String) {
        match self {
            AddField::Alias => draft.alias = value,
            AddField::Host => draft.host = value,
            AddField::User => draft.user = value,
            AddField::Port => draft.port = value,
            AddField::Key => draft.key_path = value,
            AddField::Folder => draft.folder = value,
        }
    }
}

/// The SSH default port, spelled once.
const DEFAULT_PORT: u16 = 22;

/// Where the cursor sits in a form map.
///
/// The submit row is a row the cursor can be **on**, not a control parked
/// outside the map. That is what lets `Enter` obey one rule — *act on the
/// row you are standing on* — instead of meaning "advance" everywhere and
/// "commit" somewhere else, and it is what makes the button reachable with
/// the same arrows the fields are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormCursor {
    Field(AddField),
    Submit,
}

impl FormCursor {
    /// Fields plus the submit row.
    pub const ROWS: usize = AddField::ORDER.len() + 1;

    /// The row's position, top to bottom.
    pub fn index(self) -> usize {
        match self {
            FormCursor::Field(f) => AddField::ORDER
                .iter()
                .position(|x| *x == f)
                .unwrap_or(0),
            FormCursor::Submit => AddField::ORDER.len(),
        }
    }

    fn from_index(i: usize) -> Self {
        if i >= AddField::ORDER.len() {
            FormCursor::Submit
        } else {
            FormCursor::Field(AddField::ORDER[i])
        }
    }

    /// One row down, stopping at the bottom rather than wrapping.
    ///
    /// `↓` saturates because it is the *reading* key: a user walking the
    /// map with it expects to stop at the end, not to be teleported to the
    /// top and have to read the whole thing again to find where they were.
    pub fn down(self) -> Self {
        Self::from_index((self.index() + 1).min(Self::ROWS - 1))
    }

    /// One row up, stopping at the top.
    pub fn up(self) -> Self {
        Self::from_index(self.index().saturating_sub(1))
    }

    /// One row down, wrapping top-from-bottom.
    ///
    /// `Tab` wraps because it is the *travel* key: on a seven-row map a
    /// Tab that dies at the end is a Tab the user has to count.
    pub fn next(self) -> Self {
        Self::from_index((self.index() + 1) % Self::ROWS)
    }

    /// One row up, wrapping bottom-from-top.
    pub fn prev(self) -> Self {
        Self::from_index((self.index() + Self::ROWS - 1) % Self::ROWS)
    }

    pub fn is_submit(self) -> bool {
        matches!(self, FormCursor::Submit)
    }
}

/// What a map row's leading glyph says about its field.
///
/// Five states, each with its own glyph, because the frame owns no
/// background and a hue alone vanishes under `NO_COLOR`. The set is
/// deliberately *ordinal*: `○` and `·` are both "nothing here" but say
/// different things about whether that is a problem, and collapsing them
/// is how a required field starts looking optional.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowGlyph {
    /// Filled and valid.
    Valid,
    /// Valid, and different from what the stored Connection holds.
    Changed,
    /// Required and still empty.
    Needed,
    /// Has content that will not validate.
    Invalid,
    /// Optional and empty.
    Empty,
}

impl RowGlyph {
    pub fn char(self) -> char {
        match self {
            RowGlyph::Valid => '\u{2713}',
            RowGlyph::Changed => '\u{25cf}',
            RowGlyph::Needed => '\u{25cb}',
            RowGlyph::Invalid => '!',
            RowGlyph::Empty => '\u{b7}',
        }
    }
}

/// One row of the form map, ready for the frame to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapRow {
    pub field: AddField,
    pub label: &'static str,
    /// Whether this field must be filled.
    ///
    /// Carried on the row because the frame needs it to pick an honest
    /// placeholder: a required field that is still empty must never be
    /// labelled `<optional>`. The real-terminal dump of the first map
    /// build showed exactly that on Alias and Host.
    pub required: bool,
    pub glyph: RowGlyph,
    /// The field's content, or `None` when it is empty — the frame then
    /// draws the mode's placeholder in the `fg_placeholder` tier.
    pub value: Option<String>,
    pub focused: bool,
}

/// Every problem with a draft, in map order, as `(label, message)`.
///
/// The error line is built from this, so "which fields are wrong" is
/// answered by the same call that paints the glyphs.
pub fn field_problems(draft: &ConnectionDraft) -> Vec<(&'static str, String)> {
    AddField::ORDER
        .iter()
        .filter_map(|f| f.problem(draft).map(|m| (f.label(), m)))
        .collect()
}

/// The draft with every field run through its own settle rule.
///
/// `None` if any field will not settle, which makes this the form's
/// validity check as well as its normaliser: trim, and the port's
/// empty-to-`22`, applied at the one moment the value becomes a write.
pub fn settled_draft(draft: &ConnectionDraft) -> Option<ConnectionDraft> {
    let mut out = draft.clone();
    for f in AddField::ORDER {
        match f.settle(f.field_of(draft)) {
            Ok(value) => f.commit(&mut out, value),
            Err(_) => return None,
        }
    }
    Some(out)
}

/// The map rows for a draft.
///
/// `baseline` is what the stored Connection holds. `None` on a new
/// Connection, where nothing can be *changed* — only filled.
pub fn map_rows(
    draft: &ConnectionDraft,
    focus: FormCursor,
    baseline: Option<&ConnectionDraft>,
) -> Vec<MapRow> {
    AddField::ORDER
        .iter()
        .map(|f| {
            let raw = f.field_of(draft);
            let blank = raw.trim().is_empty();
            let glyph = if !blank && f.settle(raw).is_err() {
                RowGlyph::Invalid
            } else if blank && f.required() {
                // Wins over `Changed`: clearing the Host is both, and the
                // one that blocks the save is the one worth showing.
                RowGlyph::Needed
            } else if baseline.is_some_and(|b| f.field_of(b).trim() != raw.trim()) {
                RowGlyph::Changed
            } else if blank {
                RowGlyph::Empty
            } else {
                RowGlyph::Valid
            };
            MapRow {
                field: *f,
                label: f.label(),
                required: f.required(),
                glyph,
                value: (!blank).then(|| raw.to_string()),
                focused: focus == FormCursor::Field(*f),
            }
        })
        .collect()
}

/// The refusal line: every problem with the form, joined for one row.
pub fn refusal_line(problems: &[(&'static str, String)], nothing_changed: bool) -> String {
    let mut parts: Vec<String> = problems.iter().map(|(_, msg)| msg.clone()).collect();
    if nothing_changed {
        parts.push("nothing to save".to_string());
    }
    parts.join(" \u{b7} ")
}

/// The add form as a value: where the cursor is, and what the map holds.
///
/// The draft is **live** — every keystroke writes straight into the field
/// the cursor is on. That is what lets the map show all six fields at once
/// from one source of truth instead of one step at a time, and it is why
/// there is no separate `input` line that has to be settled before the row
/// stops lying: the row *is* the draft.
///
/// Normalisation is deliberately **not** applied while typing. Showing `22`
/// in a field the user left empty would be putting words in their mouth;
/// the map shows what was typed, and [`settled_draft`] applies the
/// defaults at the moment the value becomes a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddSequence {
    /// Which row of the map the cursor is on.
    pub cursor: FormCursor,
    /// The form's live content. Nothing here has been written anywhere:
    /// the draft becomes a Connection only when the submit row is
    /// accepted, and even then the store decides whether it exists.
    pub draft: ConnectionDraft,
    /// The last refusal, drawn in the rule row. Cleared by the next
    /// keystroke that could fix it, so a mistake stops being reported the
    /// moment the user starts correcting it.
    pub error: Option<String>,
}

impl AddSequence {
    /// A blank form with the cursor on the first field.
    pub fn start() -> Self {
        Self {
            cursor: FormCursor::Field(AddField::Alias),
            draft: ConnectionDraft::default(),
            error: None,
        }
    }

    /// The map the frame draws.
    pub fn rows(&self) -> Vec<MapRow> {
        map_rows(&self.draft, self.cursor, None)
    }

    /// Whether the `\u{25b6} Add connection` row would be accepted now.
    pub fn ready(&self) -> bool {
        settled_draft(&self.draft).is_some()
    }

    /// Every problem with the form, in map order.
    pub fn problems(&self) -> Vec<(&'static str, String)> {
        field_problems(&self.draft)
    }

    /// Type into the field under the cursor.
    ///
    /// On the submit row typing does nothing. Appending to whichever field
    /// was last focused would be the worse failure: the user would be
    /// changing a row they are not looking at.
    fn typed(mut self, ch: char) -> Self {
        if let FormCursor::Field(field) = self.cursor {
            let mut value = field.field_of(&self.draft).to_string();
            value.push(ch);
            field.commit(&mut self.draft, value);
            self.error = None;
        }
        self
    }

    /// Delete one character from the field under the cursor.
    fn deleted(mut self) -> Self {
        if let FormCursor::Field(field) = self.cursor {
            let mut value = field.field_of(&self.draft).to_string();
            value.pop();
            field.commit(&mut self.draft, value);
            self.error = None;
        }
        self
    }
}

/// The edit form as a value: the same map the add form draws, seeded from
/// a Connection that already exists.
///
/// **This is the add flow, not a second editor.** `Ctrl+E` opens the same
/// six-row map, the same cursor, the same ▶ row — the differences are
/// that the rows arrive filled and the button says *Save* and names the
/// Connection. Two flows that look alike and behave differently are the
/// thing users get wrong; one flow with a different starting draft is not.
///
/// **This deliberately departs from #31 story 26**, which asked that an
/// edit "change exactly one field". Here Enter advances and the ▶ row
/// commits, so one save can carry several changed fields. The old shape
/// committed on every Enter, which meant a two-field correction wrote the
/// file twice and could not be abandoned halfway through. Changed fields
/// wear ● so the save is reviewable before it happens.
/// See `docs/adr/0001-form-map-replaces-the-stepped-add.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSequence {
    /// The Connection being edited, captured by value at the chord.
    ///
    /// Carrying the whole value is what lets the frame name the target on
    /// the button, lets each field be pre-filled from what it actually
    /// holds, and lets the driver hand the store the id of the thing the
    /// user was looking at rather than whatever the cursor has drifted
    /// onto since.
    pub target: Connection,
    /// Which row of the map the cursor is on.
    pub cursor: FormCursor,
    /// The live copy, seeded from `target`.
    pub draft: ConnectionDraft,
    /// The untouched copy the draft is diffed against.
    ///
    /// Kept as a value rather than recomputed from `target` so the diff has
    /// one obvious source and cannot drift from what the map showed when
    /// the form opened.
    pub baseline: ConnectionDraft,
    /// The last refusal, drawn in the rule row.
    pub error: Option<String>,
}

impl EditSequence {
    /// Open the map on a Connection, cursor on the first field.
    ///
    /// The cursor starts at the top rather than on the field the user is
    /// most likely to change: guessing wrong costs a keystroke either way,
    /// and a predictable starting row beats a clever one the user cannot
    /// learn.
    pub fn start(target: Connection) -> Self {
        let draft = ConnectionDraft::from_connection(&target);
        Self {
            cursor: FormCursor::Field(AddField::Alias),
            baseline: draft.clone(),
            draft,
            target,
            error: None,
        }
    }

    /// The map the frame draws, with the stored Connection as the diff
    /// baseline.
    pub fn rows(&self) -> Vec<MapRow> {
        map_rows(&self.draft, self.cursor, Some(&self.baseline))
    }

    /// The fields that differ from the stored Connection, in map order.
    ///
    /// Compared trimmed: typing three spaces into an empty optional is not
    /// a change to the file, and calling it one would let the button light
    /// up for a save that changes nothing.
    pub fn changed(&self) -> Vec<AddField> {
        AddField::ORDER
            .iter()
            .filter(|f| f.field_of(&self.draft).trim() != f.field_of(&self.baseline).trim())
            .copied()
            .collect()
    }

    /// Whether the ▶ Save row would be accepted now: nothing invalid, and
    /// something actually changed.
    pub fn ready(&self) -> bool {
        !self.changed().is_empty() && settled_draft(&self.draft).is_some()
    }

    /// Every problem with the form, in map order.
    pub fn problems(&self) -> Vec<(&'static str, String)> {
        field_problems(&self.draft)
    }

    /// Type into the field under the cursor. See [`AddSequence::typed`]
    /// for why the submit row swallows it.
    fn typed(mut self, ch: char) -> Self {
        if let FormCursor::Field(field) = self.cursor {
            let mut value = field.field_of(&self.draft).to_string();
            value.push(ch);
            field.commit(&mut self.draft, value);
            self.error = None;
        }
        self
    }

    /// Delete one character from the field under the cursor.
    fn deleted(mut self) -> Self {
        if let FormCursor::Field(field) = self.cursor {
            let mut value = field.field_of(&self.draft).to_string();
            value.pop();
            field.commit(&mut self.draft, value);
            self.error = None;
        }
        self
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
    /// The import ran **and the store really wrote `imported`
    /// Connections**; `failed` stanzas could not become Connections at
    /// all (#38).
    ///
    /// Earned through [`settle_import`], never by the `y` that answered
    /// the offer — the same discipline [`Trace::Deleted`] and
    /// [`Trace::Added`] are held to. `◇ imported 12 connections` is a
    /// claim about `connections.json`, and the keystroke that asked for
    /// it knows nothing about the file.
    ///
    /// Both numbers travel because a partial import is not a whole one.
    /// The skipped count names stanzas the scan already left out of the
    /// offer — a `Host *.example.com` can never become a Connection — so
    /// it is not a shortfall of what the user was promised; it is the
    /// rest of the file, reported rather than silently dropped. The live
    /// PTY run for #38 is why the word is *skipped* and not *failed*:
    /// `1 failed` under an offer of `3` read as one of the three having
    /// failed, when nothing of the three had.
    Imported { imported: usize, failed: usize },
    /// The offer was answered no, or abandoned. Nothing was written
    /// (#38).
    ///
    /// The import half of [`Trace::AddAbandoned`], with one difference
    /// worth naming: the offer is the only ask the user did not raise, so
    /// the decline is the answer the frame is most likely to show. It
    /// exists so the empty list underneath it reads as *heard and
    /// refused* rather than as a screen that never asked anything.
    ImportDeclined,
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
    /// Fold the ssh config at `path` into the store and persist the
    /// result (#38).
    ///
    /// The path travels with the request because the user agreed to
    /// *that* file: the offer named it, and the write has to be about
    /// the same one. What arrives is not known here — the store answers
    /// with an [`ImportReport`], and [`settle_import`] is where the
    /// frame learns whether it may say `imported`.
    ///
    /// [`ImportReport`]: crate::config::ImportReport
    Import { path: String },
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

/// What a [`Store::import_ssh_config`] call actually did (#38).
///
/// The import half of [`DeleteOutcome`]. The state machine owns no disk
/// and cannot know how many Connections a file holds, so the driver runs
/// the effect and hands the answer back through [`settle_import`].
///
/// There are two arms where the delete has three because an import has
/// no *absent* case. A file with nothing to import answers truthfully
/// with `imported: 0` — nothing was written, and nothing on screen
/// contradicts that — whereas `Ok(None)` from a delete is a claim the
/// frame could not otherwise tell from a removal.
///
/// [`Store::import_ssh_config`]: crate::connections::Store::import_ssh_config
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportOutcome {
    /// The import ran. `imported` Connections were written; `failed`
    /// stanzas could not become Connections.
    Imported { imported: usize, failed: usize },
    /// The store refused the import and said why.
    Failed(String),
}

impl ImportOutcome {
    /// Classify what [`Store::import_ssh_config`] returned.
    ///
    /// The report's two halves become the note's two numbers, with
    /// `failed` read through [`ImportReport::failed`] rather than
    /// recounted here — one place that says how many stanzas did not
    /// cross, so the note and the report cannot drift apart.
    ///
    /// [`Store::import_ssh_config`]: crate::connections::Store::import_ssh_config
    /// [`ImportReport::failed`]: crate::config::ImportReport::failed
    pub fn from_report(result: Result<config::ImportReport, String>) -> Self {
        match result {
            Ok(report) => ImportOutcome::Imported {
                imported: report.imported,
                failed: report.failed(),
            },
            Err(message) => ImportOutcome::Failed(message),
        }
    }
}

/// Fold the store's answer to [`Effect::Import`] into the state the frame
/// is rebuilt from (#38).
///
/// The import half of the honesty rule, mirroring [`settle_add`] and
/// [`settle_delete`]:
///
/// * `Imported { imported, failed }` — the note earns `imported`, with
///   the numbers the store reported rather than the count the offer was
///   made at. Those can differ: the scan and the write are two reads of
///   the same file, and the file is allowed to have changed in between.
/// * `Failed(message)` — the caller collapses the frame to `◆ error …`,
///   the same way a refused add and a refused delete do. A live list the
///   store cannot vouch for is worse than no list.
///
/// The refresh is the one [`settle_add`] performs — filter cleared,
/// cursor at the top — and the reason is stronger here than anywhere
/// else: the offer only ever appears on an empty set, so a filter left
/// over from before it would hide every Connection the user just said
/// yes to, and the frame would read `No matches` under a note naming
/// twelve.
pub fn settle_import(_state: &ManageState, outcome: ImportOutcome) -> Result<ManageState, String> {
    match outcome {
        ImportOutcome::Imported { imported, failed } => Ok(ManageState {
            phase: Phase::List,
            query: String::new(),
            selection: 0,
            trace: Some(Trace::Imported { imported, failed }),
        }),
        ImportOutcome::Failed(message) => Err(message),
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
        // `count` is not re-read here: the offer's answer is about the
        // file, and what arrived from it is the store's answer, not the
        // scan's estimate.
        Phase::ConfirmImport { ref path, .. } => confirm_import_step(state, key, path),
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

/// The first-run import offer: `y` imports, everything else declines (#38).
///
/// The answer set is [`confirm_step`]'s, unchanged, because the same
/// argument applies with more force — this is the only ask the user did
/// not raise themselves:
///
/// * `y` / `Y` — accept. One import, of the path the offer named.
/// * `n` / `N`, `Esc` — decline. Nothing is asked for.
/// * any other printable — decline **and** filter. The filter is live
///   right through the offer (user story 21), so a user who did not
///   mean to answer a question that appeared on its own and carries on
///   typing ends up searching, not importing twelve Connections they
///   never asked for. The character is not wasted and is not an answer.
/// * anything else — ignored. Movement in particular does nothing: the
///   offer is about a file, not about a row, and there is no row to
///   point at anyway.
fn confirm_import_step(state: &ManageState, key: KeyEvent, path: &str) -> Step {
    if is_chord(key, 'c') {
        return Step::exit(state.clone(), InlineOutcome::Cancelled);
    }

    if is_answer(key, 'y') {
        // The ask, not the claim. `trace` deliberately stays empty:
        // how many Connections the file holds is a fact about the disk,
        // and the disk has not been asked yet. The driver runs the
        // effect and folds the answer back through [`settle_import`],
        // which is what puts `imported` — or the collapse — on the
        // frame.
        return Step {
            state: ManageState {
                phase: Phase::List,
                trace: None,
                ..state.clone()
            },
            effects: vec![Effect::Import {
                path: path.to_string(),
            }],
        };
    }

    if is_answer(key, 'n') || key.code == KeyCode::Esc {
        return Step::state_only(import_declined(state));
    }

    if let KeyCode::Char(ch) = key.code {
        if key.modifiers.is_empty() {
            let mut next = import_declined(state);
            next.query.push(ch);
            return Step::state_only(next);
        }
    }

    Step::state_only(state.clone())
}

/// The state after an offer that did not import: back on the list, with
/// the decline left as the trace.
///
/// The import half of [`declined`], minus the target — a decline is a
/// decline whatever the file was, and the frame says `import declined`
/// rather than naming a path the user has already seen.
fn import_declined(state: &ManageState) -> ManageState {
    ManageState {
        phase: Phase::List,
        trace: Some(Trace::ImportDeclined),
        ..state.clone()
    }
}

/// The add form: the whole map on screen, one row under the cursor.
///
/// Three rules the whole form is built on:
///
/// * **Typing edits the field under the cursor, never the filter behind
///   it.** The filter is what the user returns to; the form borrows the
///   frame, not their search.
/// * **Movement is never refused.** `\u{2191}`/`\u{2193}` walk the rows, `Tab` travels
///   with wrap, and `Enter` advances off any field. A field that will not
///   validate still moves the cursor, because its `!` glyph and the dim
///   `\u25b6` row already say what is wrong — trapping the user on the row
///   to add safety they already have is the thing this map was drawn to
///   remove.
/// * **Nothing is written until the `\u25b6` row is accepted**, and even then
///   the form only *asks* — [`settle_add`] is where the frame learns
///   whether the write happened.
fn add_step(state: &ManageState, key: KeyEvent, sequence: &AddSequence) -> Step {
    if is_chord(key, 'c') {
        return Step::exit(state.clone(), InlineOutcome::Cancelled);
    }

    // Every cursor move produces the same shape; `on` wraps it so no arm
    // re-types the spread.
    let on = |cursor: FormCursor| {
        Step::state_only(ManageState {
            phase: Phase::Add(AddSequence {
                cursor,
                ..sequence.clone()
            }),
            ..state.clone()
        })
    };

    match key.code {
        KeyCode::Enter => {
            if sequence.cursor.is_submit() {
                return submit_add(state, sequence);
            }
            on(sequence.cursor.next())
        }
        KeyCode::Down => on(sequence.cursor.down()),
        KeyCode::Up => on(sequence.cursor.up()),
        KeyCode::Tab => on(sequence.cursor.next()),
        KeyCode::BackTab => on(sequence.cursor.prev()),
        KeyCode::Esc => {
            // Esc is the escape hatch, not a second arrow: from any row it
            // backs up one, and from the top row it leaves the form. Backing
            // off the first row is backing out of the whole thing, and what
            // the user needs answered is whether the half-typed form got
            // saved. It did not — no `Effect::Add` leaves this module until
            // the `\u25b6` row is accepted — and the note says so.
            if sequence.cursor.index() == 0 {
                Step::state_only(ManageState {
                    phase: Phase::List,
                    trace: Some(Trace::AddAbandoned),
                    ..state.clone()
                })
            } else {
                on(sequence.cursor.up())
            }
        }
        KeyCode::Backspace => Step::state_only(ManageState {
            phase: Phase::Add(sequence.clone().deleted()),
            ..state.clone()
        }),
        KeyCode::Char(ch) if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() => {
            Step::state_only(ManageState {
                phase: Phase::Add(sequence.clone().typed(ch)),
                ..state.clone()
            })
        }
        _ => Step::state_only(state.clone()),
    }
}

/// Accept or refuse the `\u25b6 Add connection` row.
///
/// The one keystroke in the add form that asks for a write, and the only
/// place validation gates anything. A refusal keeps the whole form and adds
/// the reason: every field that is wrong, in map order, on one line.
fn submit_add(state: &ManageState, sequence: &AddSequence) -> Step {
    match settled_draft(&sequence.draft) {
        Some(draft) => Step {
            state: ManageState {
                phase: Phase::List,
                trace: None,
                ..state.clone()
            },
            effects: vec![Effect::Add { draft }],
        },
        None => {
            let problems = sequence.problems();
            Step::state_only(ManageState {
                phase: Phase::Add(AddSequence {
                    error: Some(refusal_line(&problems, false)),
                    ..sequence.clone()
                }),
                ..state.clone()
            })
        }
    }
}

/// The edit form: the same map, seeded from a live Connection.
///
/// The same three rules [`add_step`] is built on, plus one:
///
/// * **Movement changes which row is under the cursor, never which
///   Connection is being edited.** The target was captured at the chord.
///   Arrowing across six rows and a button cannot move the edit onto a
///   neighbour, for the same reason a cursor cannot move a delete onto one:
///   the user answered about the thing they were looking at.
fn edit_step(state: &ManageState, key: KeyEvent, editor: &EditSequence) -> Step {
    if is_chord(key, 'c') {
        return Step::exit(state.clone(), InlineOutcome::Cancelled);
    }

    let on = |cursor: FormCursor| {
        Step::state_only(ManageState {
            phase: Phase::Edit(EditSequence {
                cursor,
                ..editor.clone()
            }),
            ..state.clone()
        })
    };

    match key.code {
        KeyCode::Enter => {
            if editor.cursor.is_submit() {
                return submit_edit(state, editor);
            }
            on(editor.cursor.next())
        }
        KeyCode::Down => on(editor.cursor.down()),
        KeyCode::Up => on(editor.cursor.up()),
        KeyCode::Tab => on(editor.cursor.next()),
        KeyCode::BackTab => on(editor.cursor.prev()),
        KeyCode::Esc => {
            // Backing out of an edit answers the same question the add
            // form answers: did the half-typed change get saved? It did
            // not — no `Effect::Update` leaves this module until the
            // `\u25b6` row is accepted — so Esc from the top row can say so
            // honestly, whatever the user had typed into the rows below it.
            if editor.cursor.index() == 0 {
                Step::state_only(ManageState {
                    phase: Phase::List,
                    trace: Some(Trace::EditAbandoned),
                    ..state.clone()
                })
            } else {
                on(editor.cursor.up())
            }
        }
        KeyCode::Backspace => Step::state_only(ManageState {
            phase: Phase::Edit(editor.clone().deleted()),
            ..state.clone()
        }),
        KeyCode::Char(ch) if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() => {
            Step::state_only(ManageState {
                phase: Phase::Edit(editor.clone().typed(ch)),
                ..state.clone()
            })
        }
        _ => Step::state_only(state.clone()),
    }
}

/// Accept or refuse the `\u25b6 Save changes to <alias>` row.
///
/// Two things can stop it, and both are said on the same line: a field that
/// will not validate, and a form that has not actually changed. The second
/// is an edit-only refusal — an add with nothing filled is simply not
/// ready, but an edit that has touched nothing has answered a question
/// nobody asked, and silently writing the file back would let the user
/// believe something was saved.
fn submit_edit(state: &ManageState, editor: &EditSequence) -> Step {
    let problems = editor.problems();
    let nothing_changed = editor.changed().is_empty();
    let settled = if problems.is_empty() {
        settled_draft(&editor.draft)
    } else {
        None
    };

    match (settled, nothing_changed) {
        (Some(draft), false) => Step {
            state: ManageState {
                phase: Phase::List,
                trace: None,
                ..state.clone()
            },
            effects: vec![Effect::Update {
                target: editor.target.clone(),
                draft,
            }],
        },
        _ => Step::state_only(ManageState {
            phase: Phase::Edit(EditSequence {
                error: Some(refusal_line(&problems, nothing_changed)),
                ..editor.clone()
            }),
            ..state.clone()
        }),
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
