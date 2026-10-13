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

use crate::config::{Connection, Source};
use crate::connections::{self, ConnectionDraft};
use crate::inline::InlineOutcome;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What the manage frame is showing.
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
    /// The inline adopt ask: copy this Config Host into sshm's store.
    ///
    /// Enter over a config row, because a row merged from `~/.ssh/config` is
    /// the user's own stanza and the one thing sshm may do with it is offer a
    /// copy. It is a confirm and not an instant write for the same reason a
    /// delete is: the answer writes `connections.json`, and a keystroke that
    /// means "this one" must not be a file change on its own.
    ///
    /// `target` is the row Enter was pressed over, captured by value exactly as
    /// [`Phase::ConfirmDelete`] captures its target — including its
    /// `ssh-config:` id, which is what the store refuses to delete and the
    /// thing an adopt copies *from* rather than into.
    ConfirmAdopt { target: Connection },
    /// The Form Map, open on one of its two starting drafts (#37).
    ///
    /// One value holds the whole surface: which row the cursor is on, what
    /// the map holds, what the last refusal said, and — for `Ctrl+E` — the
    /// Connection it was opened from. That is what makes "what does Enter
    /// do on the Port row" a question a unit test can answer with no
    /// terminal, and it is why there is one phase rather than an add phase
    /// and an edit phase: `Ctrl+A` and `Ctrl+E` are two ways of opening the
    /// same map, not two surfaces.
    ///
    /// The Connection a `Ctrl+E` map is about is captured by value at the
    /// chord, exactly as [`Phase::ConfirmDelete`] captures its target.
    /// Everything the map then does is about *this* Connection; a cursor
    /// that drifts afterwards changes nothing about it.
    ///
    /// Boxed because a live draft, the Connection it came from and the copy
    /// it is diffed against is the largest thing a `Phase` carries, and the
    /// frame holds one on its state rather than passing phases around.
    Form(Box<FormMap>),
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
///   which is what `connections::connection_from` falls back to. Anything
///   that is not a number in `1..=65535` is rejected, because a `port`
///   that silently becomes `22` after the user typed `99999` is a
///   Connection that connects to the wrong machine.
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
pub enum FormField {
    Alias,
    Host,
    User,
    Port,
    Key,
    Folder,
}

impl FormField {
    /// The order the map walks, top to bottom.
    pub const ORDER: [FormField; 6] = [
        FormField::Alias,
        FormField::Host,
        FormField::User,
        FormField::Port,
        FormField::Key,
        FormField::Folder,
    ];

    /// The field's label — the word that wears the `◆` when its row is
    /// focused and the `◇` once it has settled.
    pub fn label(self) -> &'static str {
        match self {
            FormField::Alias => "Alias",
            FormField::Host => "Host",
            FormField::User => "User",
            FormField::Port => "Port",
            FormField::Key => "Key",
            FormField::Folder => "Folder",
        }
    }

    /// Whether this field must be answered with something.
    fn required(self) -> bool {
        matches!(self, FormField::Alias | FormField::Host)
    }

    /// Validate and normalise one row's answer.
    ///
    /// `Ok(value)` is what settles into the draft — trimmed, and for the
    /// port row the empty answer already replaced with the SSH default.
    /// `Err(message)` is the sentence the frame shows while the user stays
    /// on this row with what they typed still there.
    ///
    /// The empty-answer rule is asked of [`FormField::required`], not
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
                FormField::Port => Ok(DEFAULT_PORT.to_string()),
                _ => Ok(String::new()),
            };
        }

        match self {
            FormField::Port => match value.parse::<u16>() {
                // Short on purpose. The full sentence ("must be a number
                // from 1 to 65535") plus the example plus two other
                // problems on the line overruns 80 columns; this leaves
                // room for all three problems *and* the example.
                Ok(0) | Err(_) => Err("port must be 1–65535 (e.g. 22)".to_string()),
                Ok(port) => Ok(port.to_string()),
            },
            // The one folder name sshm keeps for itself: a stored row filed
            // under `ssh-config` would draw the tag a Config Host draws and
            // read as a row nobody may edit. The store refuses the same draft
            // ([`crate::connections::is_reserved_folder`]); asking it here is
            // what puts the `!` on the row where it was typed instead of
            // letting the write fail three keystrokes later.
            FormField::Folder => {
                if connections::is_reserved_folder(value) {
                    Err(connections::reserved_folder_refusal(value))
                } else {
                    Ok(value.to_string())
                }
            }
            _ => Ok(value.to_string()),
        }
    }

    /// The value this field holds in a draft, as a borrow.
    ///
    /// The one place that names which draft member a field *is*. The glyphs,
    /// the diff and the settled draft all read through it, so they can never
    /// disagree about what `Port` means.
    fn field_of(self, draft: &ConnectionDraft) -> &str {
        match self {
            FormField::Alias => &draft.alias,
            FormField::Host => &draft.host,
            FormField::User => &draft.user,
            FormField::Port => &draft.port,
            FormField::Key => &draft.key_path,
            FormField::Folder => &draft.folder,
        }
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
            FormField::Alias => draft.alias = value,
            FormField::Host => draft.host = value,
            FormField::User => draft.user = value,
            FormField::Port => draft.port = value,
            FormField::Key => draft.key_path = value,
            FormField::Folder => draft.folder = value,
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
    Field(FormField),
    Submit,
}

impl FormCursor {
    /// Fields plus the submit row.
    pub const ROWS: usize = FormField::ORDER.len() + 1;

    /// The row's position, top to bottom.
    pub fn index(self) -> usize {
        match self {
            FormCursor::Field(f) => FormField::ORDER.iter().position(|x| *x == f).unwrap_or(0),
            FormCursor::Submit => FormField::ORDER.len(),
        }
    }

    fn from_index(i: usize) -> Self {
        if i >= FormField::ORDER.len() {
            FormCursor::Submit
        } else {
            FormCursor::Field(FormField::ORDER[i])
        }
    }

    /// One row down, stopping at the bottom rather than wrapping.
    ///
    /// `↓` saturates because it is the *reading* key: a user walking the
    /// map with it expects to stop at the end, not to be teleported to the
    /// top and have to read the whole thing again to find where they were.
    ///
    /// Saturating needs no clamp: [`Self::from_index`] already calls every
    /// index past the last field the submit row, and one past the submit row
    /// is still past it. A `.min` here would be arithmetic that looks like it
    /// protects the bottom row and in fact cannot be reached.
    pub fn down(self) -> Self {
        Self::from_index(self.index() + 1)
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
    pub field: FormField,
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
fn field_problems(draft: &ConnectionDraft) -> Vec<(&'static str, String)> {
    FormField::ORDER
        .iter()
        .filter_map(|f| f.problem(draft).map(|m| (f.label(), m)))
        .collect()
}

/// The draft with every field run through its own settle rule.
///
/// `None` if any field will not settle, which makes this the form's
/// validity check as well as its normaliser: trim, and the port's
/// empty-to-`22`, applied at the one moment the value becomes a write.
fn settled_draft(draft: &ConnectionDraft) -> Option<ConnectionDraft> {
    let mut out = draft.clone();
    for f in FormField::ORDER {
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
fn map_rows(
    draft: &ConnectionDraft,
    focus: FormCursor,
    baseline: Option<&ConnectionDraft>,
) -> Vec<MapRow> {
    FormField::ORDER
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
fn refusal_line(problems: &[(&'static str, String)], nothing_changed: bool) -> String {
    let mut parts: Vec<String> = problems.iter().map(|(_, msg)| msg.clone()).collect();
    if nothing_changed {
        parts.push("nothing to save".to_string());
    }
    parts.join(" \u{b7} ")
}

/// The Form Map as a value: where the cursor is, and what the map holds.
///
/// **One surface with two starting drafts.** `Ctrl+A` opens it blank,
/// `Ctrl+E` opens it seeded from a Connection that already exists. There is
/// no second map: the differences are that the rows arrive filled, the
/// ▶ row says *Save* and names the Connection, and a save that changes
/// nothing is refused. Two flows that look alike and behave differently are
/// the thing users get wrong; one flow with a different starting draft is
/// not.
///
/// The draft is **live** — every keystroke writes straight into the field
/// the cursor is on. That is what lets the map show all six fields at once
/// from one source of truth instead of one field at a time, and it is why
/// there is no separate `input` line that has to be settled before the row
/// stops lying: the row *is* the draft.
///
/// Normalisation is deliberately **not** applied while typing. Showing `22`
/// in a field the user left empty would be putting words in their mouth;
/// the map shows what was typed, and `settled_draft` applies the defaults
/// at the moment the value becomes a write.
///
/// **An edit departs from #31 story 26**, which asked that an edit "change
/// exactly one field". Here Enter advances and the ▶ row commits, so one
/// save can carry several changed fields. The old shape committed on every
/// Enter, which meant a two-field correction wrote the file twice and could
/// not be abandoned halfway through. Changed fields wear ● so the save is
/// reviewable before it happens.
/// See `docs/adr/0001-form-map-replaces-the-stepped-add.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormMap {
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
    /// What the map was opened *from*, when it was opened from something.
    ///
    /// One field rather than two, because a target and the baseline diffed
    /// against it are always present or absent together, and that pairing
    /// *is* the add/edit distinction. Two sibling fields would let a state
    /// exist that the frame has no honest way to draw.
    stored: Option<StoredDraft>,
}

/// The stored Connection a map was opened from, and the copy it diffs
/// against.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredDraft {
    /// Captured by value at the chord. Carrying the whole value is what lets
    /// the frame name the target on the ▶ row, lets each field be pre-filled
    /// from what it actually holds, and lets the driver hand the store the
    /// id of the thing the user was looking at rather than whatever the
    /// cursor has drifted onto since.
    target: Connection,
    /// The untouched copy the draft is diffed against.
    ///
    /// Kept as a value rather than recomputed from `target` so the diff has
    /// one obvious source and cannot drift from what the map showed when it
    /// opened.
    baseline: ConnectionDraft,
}

impl FormMap {
    /// Open a blank map, cursor on the first field (`Ctrl+A`).
    pub fn start() -> Self {
        Self {
            cursor: FormCursor::Field(FormField::Alias),
            draft: ConnectionDraft::default(),
            error: None,
            stored: None,
        }
    }

    /// Open the map on a Connection, cursor on the first field (`Ctrl+E`).
    ///
    /// The cursor starts at the top rather than on the field the user is
    /// most likely to change: guessing wrong costs a keystroke either way,
    /// and a predictable starting row beats a clever one the user cannot
    /// learn.
    pub fn from_connection(target: &Connection) -> Self {
        let draft = ConnectionDraft::from_connection(target);
        Self {
            cursor: FormCursor::Field(FormField::Alias),
            draft: draft.clone(),
            error: None,
            stored: Some(StoredDraft {
                target: target.clone(),
                baseline: draft,
            }),
        }
    }

    /// The Connection being edited, or `None` for a new one.
    pub fn target(&self) -> Option<&Connection> {
        self.stored.as_ref().map(|stored| &stored.target)
    }

    /// The map the frame draws, diffed against the stored Connection when
    /// there is one.
    pub fn rows(&self) -> Vec<MapRow> {
        map_rows(
            &self.draft,
            self.cursor,
            self.stored.as_ref().map(|stored| &stored.baseline),
        )
    }

    /// The fields that differ from the stored Connection, in map order.
    ///
    /// Empty for a new Connection: there is nothing to differ from, and an
    /// add is never refused for having changed nothing.
    ///
    /// Compared trimmed: typing three spaces into an empty optional is not
    /// a change to the file, and calling it one would let the ▶ row light
    /// up for a save that changes nothing.
    pub fn changed(&self) -> Vec<FormField> {
        let Some(stored) = &self.stored else {
            return Vec::new();
        };
        FormField::ORDER
            .iter()
            .filter(|f| f.field_of(&self.draft).trim() != f.field_of(&stored.baseline).trim())
            .copied()
            .collect()
    }

    /// Whether the ▶ row would be accepted now: nothing invalid, and — for
    /// a map opened from a Connection — something actually changed.
    pub fn ready(&self) -> bool {
        settled_draft(&self.draft).is_some()
            && (self.stored.is_none() || !self.changed().is_empty())
    }

    /// Every problem with the form, in map order.
    pub fn problems(&self) -> Vec<(&'static str, String)> {
        field_problems(&self.draft)
    }

    /// Whether the ▶ row is being asked to save a map that has not changed.
    ///
    /// This is an edit-only refusal: an add with nothing filled is simply
    /// not ready, but an edit that has touched nothing has answered a
    /// question nobody asked, and silently writing the file back would let
    /// the user believe something was saved.
    fn nothing_changed(&self) -> bool {
        self.stored.is_some() && self.changed().is_empty()
    }

    /// The words on the ▶ row. Built here, where the target is in hand,
    /// rather than in the renderer, so the row and the header cannot drift
    /// into disagreeing about which Connection this is.
    pub fn submit_label(&self) -> String {
        match self.target() {
            Some(target) => format!("Save changes to {}", target.alias),
            None => "Add connection".to_string(),
        }
    }

    /// The header's words.
    pub fn header(&self) -> String {
        match self.target() {
            // The folder is part of the name the user picked the Connection
            // by: two Connections can share an alias across folders, and
            // "Edit web" over two of them answers nothing.
            Some(target) => match target.folder.as_deref().filter(|folder| !folder.is_empty()) {
                Some(folder) => format!("Edit [{folder}] {}", target.alias),
                None => format!("Edit {}", target.alias),
            },
            None => "New connection".to_string(),
        }
    }

    /// What an empty field shows in place of a value.
    ///
    /// A blank map says `<optional>`: an invitation. A seeded one says
    /// `<not set>`: a fact about the stored Connection. Same tier,
    /// different sentence, because the two are answering different
    /// questions.
    pub fn placeholder(&self) -> &'static str {
        if self.stored.is_some() {
            "<not set>"
        } else {
            "<optional>"
        }
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

/// What the last management action did.
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
    /// left the map — the same discipline [`Trace::Deleted`]
    /// is held to. `◇ added [prod] web-01` is a claim about
    /// `connections.json`, and only the store gets to make it.
    Added { connection: Connection },
    /// Enter on a Config Host was answered `y` **and the store really wrote
    /// the copy**.
    ///
    /// The adopt half of [`Trace::Added`], held to the same discipline: it is
    /// earned through [`settle_adopt`] and never by the `y` that asked, because
    /// `◇ adopted build-box` is a claim about `connections.json` and only the
    /// store gets to make it. The note names the copy rather than the stanza,
    /// because the stanza is still where it was — what changed is that sshm now
    /// has a row of its own for that box, and the config row the copy hides has
    /// left the list in the same keystroke.
    Adopted { connection: Connection },
    /// The map was backed out of without submitting. Nothing was
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
    /// The edit map was abandoned. Nothing was written.
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
    /// `Ctrl+X` or `Ctrl+E` over a Config Host: sshm will not change it, and
    /// the note says so in place of a confirm or a form.
    ///
    /// A refusal, not a settle. Nothing was asked of the store and nothing
    /// was written, so the keystroke is honest about it on its own — the
    /// discipline [`Trace::Deleted`] is held to, where only the store's answer
    /// earns the note. What it replaces is the sequence the chord would
    /// otherwise have opened: a confirm over a row that can never be deleted,
    /// or a form whose `▶` row promises a write sshm does not perform.
    ///
    /// The wording names the provenance rather than the Connection, because
    /// the reason for the refusal is where the row came from — and the row is
    /// still on the glass underneath the note, tagged `[ssh-config]`. The
    /// Connection travels with the variant all the same, because every other
    /// trace says which row it is about, and a refusal that cannot name its
    /// row is a note nothing can pin to the keystroke that raised it.
    ReadOnly {
        /// The Config Host the chord was pressed over.
        connection: Connection,
    },
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
    /// What the frame is showing.
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
    /// The frame as a bare list.
    pub fn new() -> Self {
        Self {
            query: String::new(),
            selection: 0,
            phase: Phase::List,
            trace: None,
        }
    }

    /// A list seeded with a filter.
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
    /// Emitted only from the confirm, and only for the captured target.
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
    ///   clobber a field the map never touched.
    Update {
        /// The Connection the chord was pressed over.
        target: Connection,
        /// The whole Connection as it should now stand, carrying
        /// `target`'s id.
        draft: ConnectionDraft,
    },
    /// Copy the Config Host `target` into the store and persist it.
    ///
    /// The adopt half of the write effects, emitted only from the adopt ask
    /// and only for the captured row. The whole config row travels with the
    /// request rather than its id: the id is the merge's own
    /// `ssh-config:<pattern>`, which names nothing in `connections.json`, and
    /// the fields the copy is made of are the ones on the row the user was
    /// looking at.
    Adopt { target: Connection },
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
/// map submitting is what the user *did*, and `◇ added` is a claim
/// about the file. A draft that came back `Err` never earns the word —
/// the Connection is not in `connections.json`, and a frame that said it
/// was would be describing a write that did not happen.
///
/// * `Ok(conn)` — the note earns `added`, and names the Connection the
///   store actually built (its id, its folder, its port — not what the
///   user typed into the Alias row).
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

/// Fold the store's answer to [`Effect::Adopt`] into the state the frame is
/// rebuilt from.
///
/// The adopt half of the honesty rule, the same shape as [`settle_add`]: the
/// `y` is what the user *did*, and `◇ adopted` is a claim about the file.
///
/// * `Ok(connection)` — the note earns `adopted`, and names the copy the store
///   actually wrote (its fresh id, its absent folder — not the stanza's
///   `ssh-config:` id). The list is refreshed and shows the result, which for
///   an adopt means two things at once: the new row is there and the config row
///   it hides is gone. Both are the store's doing — a `LiveStore` re-merges
///   inside `adopt` — so this only has to clear a filter that would hide the
///   new row and put the cursor on it, exactly as an add does.
/// * `Err(message)` — the store refused, and the caller collapses the frame to
///   `◆ error …` the way a refused delete does.
pub fn settle_adopt(
    _state: &ManageState,
    connections: &[Connection],
    outcome: Result<Connection, String>,
) -> Result<ManageState, String> {
    match outcome {
        Ok(connection) => {
            let selection = index_of_id(connections, &connection.id);
            Ok(ManageState {
                phase: Phase::List,
                query: String::new(),
                selection,
                trace: Some(Trace::Adopted { connection }),
            })
        }
        Err(message) => Err(message),
    }
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
///   the store actually wrote, not the one the map started from. If
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
        Phase::ConfirmAdopt { ref target } => confirm_adopt_step(state, key, target),
        Phase::Form(ref map) => form_step(state, key, map),
    }
}

/// The list: filter, movement, and the chords that start an action.
///
/// This is the whole of the manage frame's key behaviour while it is just a
/// list, so that the driver has one place to ask rather than a `match` that
/// has to remember which of these the frame is allowed to act on.
///
/// Two questions, asked in order: is this one of the chords that open a
/// sequence, and if not, what does the plain key do to the list?
fn list_step(state: &ManageState, key: KeyEvent, selected: Option<&Connection>) -> Step {
    if is_chord(key, 'c') {
        return Step::exit(state.clone(), InlineOutcome::Cancelled);
    }

    if let Some(step) = list_chord_step(state, key, selected) {
        return step;
    }

    list_key_step(state, key, selected)
}

/// The chords that open a sequence from the list: `Ctrl+X` asks to delete,
/// `Ctrl+E` opens the map for the row under the cursor, `Ctrl+A` opens an
/// empty one. `None` means the key is not one of them.
///
/// Both `Ctrl+X` and `Ctrl+E` capture their target *here*, by value:
/// everything the confirm or the map then does — including arrowing across
/// six rows — is about the Connection the user was pointing at when they
/// pressed the chord, whatever the cursor gets up to in between. With nothing
/// under the cursor they ask for nothing, because the frame will not invent a
/// target to change.
///
/// A Config Host gets neither sequence. `Ctrl+X` over a row merged from
/// `~/.ssh/config` would ask the user to confirm a delete sshm must not
/// perform, and `Ctrl+E` would hand them a form whose submit row promises a
/// write to a file sshm never writes; either one is a UI that lies. Both stop
/// at [`Trace::ReadOnly`] instead, on the list, with the filter and the cursor
/// untouched — the refusal is a note about the row, not a different screen.
fn list_chord_step(
    state: &ManageState,
    key: KeyEvent,
    selected: Option<&Connection>,
) -> Option<Step> {
    // The sequence opens on the list's own body: the filter and the cursor
    // come back with the user when it ends. What it replaces is the note,
    // never the list.
    let opened = |phase: Phase| {
        Step::state_only(ManageState {
            phase,
            ..state.clone()
        })
    };

    // A refusal is `opened` with no sequence to open: it replaces the note and
    // nothing else, so the phase rides along from `..state.clone()` — which is
    // `Phase::List`, because a chord is only read on the list. Spelling the
    // phase out here would be a claim no test could check, and the mutation
    // sweep proves it by deleting the line and losing nothing; what the tests
    // do check is that the frame is still on the list afterwards.
    let refused = |target: &Connection| {
        Step::state_only(ManageState {
            trace: Some(Trace::ReadOnly {
                connection: target.clone(),
            }),
            ..state.clone()
        })
    };

    if is_chord(key, 'x') {
        return Some(match selected {
            Some(target) if target.source == Source::SshConfig => refused(target),
            Some(target) => opened(Phase::ConfirmDelete {
                target: target.clone(),
            }),
            None => Step::state_only(state.clone()),
        });
    }

    if is_chord(key, 'e') {
        return Some(match selected {
            Some(target) if target.source == Source::SshConfig => refused(target),
            Some(target) => Step::state_only(ManageState {
                phase: Phase::Form(Box::new(FormMap::from_connection(target))),
                // The note from the last action is cleared the same way
                // `Ctrl+A` clears it: the map is a new ask, and a
                // stale `◇ deleted` sitting above "which field am I
                // editing?" reads as an answer to the wrong question.
                trace: None,
                ..state.clone()
            }),
            None => Step::state_only(state.clone()),
        });
    }

    if is_chord(key, 'a') {
        return Some(Step::state_only(ManageState {
            phase: Phase::Form(Box::new(FormMap::start())),
            trace: None,
            ..state.clone()
        }));
    }

    None
}

/// What a plain key does to the list: filter text, movement, or one of the
/// two ways out.
fn list_key_step(state: &ManageState, key: KeyEvent, selected: Option<&Connection>) -> Step {
    let mut next = state.clone();
    match key.code {
        KeyCode::Esc => return Step::exit(next, InlineOutcome::Cancelled),
        KeyCode::Enter => {
            if let Some(conn) = selected {
                if conn.source == Source::SshConfig {
                    // Enter on a Config Host asks for a copy instead of
                    // picking: the row is the user's own stanza, sshm may not
                    // change it, and the one thing worth doing to it — putting
                    // a copy in sshm's store — is a write, so it gets an ask.
                    return Step::state_only(ManageState {
                        phase: Phase::ConfirmAdopt {
                            target: conn.clone(),
                        },
                        ..next
                    });
                }
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
        KeyCode::Char(ch) if is_text(key) => next.query.push(ch),
        _ => return Step::state_only(state.clone()),
    }

    Step::state_only(next)
}

/// The confirm: `y` deletes, everything else declines.
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
        if is_text(key) {
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

/// The adopt ask: `y` copies, everything else declines and says nothing.
///
/// The answer set is [`confirm_step`]'s, because it is the same kind of
/// question — an answer that writes `connections.json` — with one difference,
/// and the difference is what the two questions are about. A declined delete
/// leaves [`Trace::Declined`] because the frame had asked the user to remove
/// one of their own Connections and they said no; a declined adopt leaves
/// **nothing**, because nothing was asked of the store and the list is exactly
/// the list it was. A settled `■` line over an unchanged row would be recording
/// a non-event.
///
/// * `y` / `Y` — confirm. One adopt, of the captured target.
/// * `n` / `N`, `Esc` — decline. No effect, no note.
/// * any other printable — decline **and** filter: a user who hits Enter by
///   accident and carries on typing ends up searching, not adopting.
/// * anything else — ignored. The target was captured at the keystroke, so
///   there is nothing for a moving cursor to change.
fn confirm_adopt_step(state: &ManageState, key: KeyEvent, target: &Connection) -> Step {
    if is_chord(key, 'c') {
        return Step::exit(state.clone(), InlineOutcome::Cancelled);
    }

    if is_answer(key, 'y') {
        // The ask, not the claim: whether a row was written is the store's to
        // say, and the driver folds its answer back through [`settle_adopt`].
        return Step {
            state: ManageState {
                phase: Phase::List,
                trace: None,
                ..state.clone()
            },
            effects: vec![Effect::Adopt {
                target: target.clone(),
            }],
        };
    }

    if is_answer(key, 'n') || key.code == KeyCode::Esc {
        return Step::state_only(declined_adopt(state));
    }

    if let KeyCode::Char(ch) = key.code {
        if is_text(key) {
            let mut next = declined_adopt(state);
            next.query.push(ch);
            return Step::state_only(next);
        }
    }

    Step::state_only(state.clone())
}

/// The state after an adopt ask that did not adopt: back on the list, with
/// nothing to report.
fn declined_adopt(state: &ManageState) -> ManageState {
    ManageState {
        phase: Phase::List,
        trace: None,
        ..state.clone()
    }
}

/// The Form Map: the whole surface on screen, one row under the cursor.
///
/// Three rules the whole form is built on:
///
/// * **Typing edits the field under the cursor, never the filter behind
///   it.** The filter is what the user returns to; the form borrows the
///   frame, not their search.
/// * **Movement is never refused.** `↑`/`↓` walk the rows, `Tab` travels
///   with wrap, and `Enter` advances off any field. A field that will not
///   validate still moves the cursor, because its `!` glyph and the dim
///   `▶` row already say what is wrong — trapping the user on the row
///   to add safety they already have is the thing this map was drawn to
///   remove.
/// * **Nothing is written until the `▶` row is accepted**, and even then
///   the form only *asks* — [`settle_add`] and [`settle_edit`] are where
///   the frame learns whether the write happened.
///
/// And one more for a map opened from a Connection: **movement changes which
/// row is under the cursor, never which Connection is being edited.** The
/// target was captured at the chord. Arrowing across six rows and a button
/// cannot move the edit onto a neighbour, for the same reason a cursor
/// cannot move a delete onto one: the user answered about the thing they
/// were looking at.
fn form_step(state: &ManageState, key: KeyEvent, map: &FormMap) -> Step {
    if is_chord(key, 'c') {
        return Step::exit(state.clone(), InlineOutcome::Cancelled);
    }

    match key.code {
        KeyCode::Enter => enter_form_step(state, map),
        KeyCode::Down => moved(state, map, map.cursor.down()),
        KeyCode::Up => moved(state, map, map.cursor.up()),
        KeyCode::Tab => moved(state, map, map.cursor.next()),
        KeyCode::BackTab => moved(state, map, map.cursor.prev()),
        KeyCode::Esc => esc_form_step(state, map),
        KeyCode::Backspace => Step::state_only(ManageState {
            phase: Phase::Form(Box::new(map.clone().deleted())),
            ..state.clone()
        }),
        KeyCode::Char(ch) if is_text(key) => Step::state_only(ManageState {
            phase: Phase::Form(Box::new(map.clone().typed(ch))),
            ..state.clone()
        }),
        _ => Step::state_only(state.clone()),
    }
}

/// The step that moves the cursor and changes nothing else.
///
/// Every cursor move produces this one shape, so no arm re-types the spread:
/// the same map with a different row under the cursor, in the same state.
fn moved(state: &ManageState, map: &FormMap, cursor: FormCursor) -> Step {
    Step::state_only(ManageState {
        phase: Phase::Form(Box::new(FormMap {
            cursor,
            ..map.clone()
        })),
        ..state.clone()
    })
}

/// Enter on the `▶` row accepts the map; anywhere else it moves along.
fn enter_form_step(state: &ManageState, map: &FormMap) -> Step {
    if map.cursor.is_submit() {
        return submit_form(state, map);
    }
    moved(state, map, map.cursor.next())
}

/// Esc is the escape hatch, not a second arrow: from any row it backs up one,
/// and from the top row it leaves the form. Backing off the first row is
/// backing out of the whole thing, and what the user needs answered is
/// whether the half-typed form got saved. It did not — no `Effect` leaves this
/// module until the `▶` row is accepted — and the note says so.
fn esc_form_step(state: &ManageState, map: &FormMap) -> Step {
    if map.cursor.index() != 0 {
        return moved(state, map, map.cursor.up());
    }

    let trace = if map.target().is_some() {
        Trace::EditAbandoned
    } else {
        Trace::AddAbandoned
    };
    Step::state_only(ManageState {
        phase: Phase::List,
        trace: Some(trace),
        ..state.clone()
    })
}

/// Accept or refuse the `▶` row.
///
/// The one keystroke on the form that asks for a write, and the only place
/// validation gates anything. A refusal keeps the whole form and adds the
/// reason: every field that is wrong, in map order, on one line. Two things
/// can stop it, and both are said on the same line — a field that will not
/// validate, and a map that has not actually changed.
fn submit_form(state: &ManageState, map: &FormMap) -> Step {
    let problems = map.problems();
    let nothing_changed = map.nothing_changed();
    // The settled draft is only asked for once nothing is already wrong with
    // the fields: `settled_draft` and `problems` are the same question asked
    // twice, and asking it once keeps the refusal honest about *why*.
    let settled = if problems.is_empty() && !nothing_changed {
        settled_draft(&map.draft)
    } else {
        None
    };

    let effect = match (settled, map.target()) {
        (Some(draft), Some(target)) => Some(Effect::Update {
            target: target.clone(),
            draft,
        }),
        (Some(draft), None) => Some(Effect::Add { draft }),
        _ => None,
    };

    match effect {
        Some(effect) => Step {
            state: ManageState {
                phase: Phase::List,
                trace: None,
                ..state.clone()
            },
            effects: vec![effect],
        },
        None => Step::state_only(ManageState {
            phase: Phase::Form(Box::new(FormMap {
                error: Some(refusal_line(&problems, nothing_changed)),
                ..map.clone()
            })),
            ..state.clone()
        }),
    }
}

/// Is this key text, as opposed to a chord?
///
/// Shift is the one modifier that is *part of typing*: a terminal sends the
/// byte `W` for a capital and crossterm reports it as `Char('W')` with SHIFT
/// set, so a rule that insisted on no modifiers at all could not type an
/// uppercase letter — a Connection named `Web-01` could not be found by typing
/// its own name, and the list filter dropped the capital in silence while the
/// form map three rows away accepted it. Every other modifier turns the key
/// into a *different question*, which is the same line [`is_chord`] reads from
/// the other side: the frame binds nothing to Alt, and a chord is not a
/// printable.
fn is_text(key: KeyEvent) -> bool {
    key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
}

/// Is this key the unadorned answer `ch` to the `(y/N)` prompt?
///
/// Shift is allowed (`Y` answers the same as `y`); every other modifier is
/// not, because a Ctrl chord is a *different question* and must not be read
/// as an answer to this one.
fn is_answer(key: KeyEvent, ch: char) -> bool {
    is_text(key) && matches!(key.code, KeyCode::Char(c) if c.eq_ignore_ascii_case(&ch))
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
