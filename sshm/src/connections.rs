//! The Connection manager.
//!
//! Every change to the Connection set lives here: add, edit and delete.
//! Each operation mutates the set, persists it when
//! something actually changed, and reports what came back, so a surface can
//! render its own trace without re-implementing the rules.
//!
//! This is business logic only. Nothing here knows how a Connection is drawn,
//! filtered or selected — a caller passes the draft it collected and the id it
//! is acting on, and gets the changed Connection out. That is what lets one
//! implementation serve every caller. Note that as of the #35 cut-over no
//! live command calls into it: `sshm add` writes through `config` directly,
//! and the manage paths that will are #36/#37. The seam is exercised by
//! `tests/connections_test.rs` until they land.

use crate::config::{Config, Connection, Source};

/// The field values a Connection is built from while the user adds or edits one.
///
/// Every field is a string because it arrives from keystroke-collected input, not
/// from a parsed config file: the port is digits-as-text, and an optional field
/// the user left alone is empty rather than absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectionDraft {
    pub alias: String,
    pub host: String,
    pub user: String,
    pub port: String,
    pub key_path: String,
    pub folder: String,
}

impl ConnectionDraft {
    /// Seed a draft from an existing Connection, so an edit starts where that
    /// Connection left off.
    pub fn from_connection(conn: &Connection) -> Self {
        Self {
            alias: conn.alias.clone(),
            host: conn.host.clone(),
            user: conn.user.clone(),
            port: conn.port.to_string(),
            key_path: conn.key_path.clone().unwrap_or_default(),
            folder: conn.folder.clone().unwrap_or_default(),
        }
    }
}

/// Build the Connection a draft describes, minting a fresh id for it.
///
/// A blank optional field is *absent*, not an empty string: a Connection with
/// `key_path: Some("")` would hand ssh `-i ""`.
///
/// The provenance is `Stored` and not carried from the draft, because a draft
/// is what the user typed into sshm's own form: whatever it describes is about
/// to become part of sshm's store, and only the code that merges a
/// `~/.ssh/config` stanza sets [`Source::SshConfig`]. A draft that could carry
/// provenance would let a form write a row that claims to be read-only.
fn connection_from(draft: &ConnectionDraft) -> Connection {
    Connection {
        id: uuid::Uuid::new_v4().to_string(),
        alias: draft.alias.clone(),
        host: draft.host.clone(),
        user: draft.user.clone(),
        port: draft.port.parse().unwrap_or(22),
        key_path: none_if_blank(&draft.key_path),
        folder: none_if_blank(&draft.folder),
        source: Source::Stored,
    }
}

fn none_if_blank(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

/// Whether `folder` is the name sshm keeps for itself.
///
/// The frame draws `[ssh-config]` for a row that came from `~/.ssh/config`,
/// from its provenance ([`crate::frame::SSH_CONFIG_TAG`]). A stored Connection
/// filed under that same word would draw the same prefix and read as a row sshm
/// must not edit, so the name is not available to a folder — and it is refused
/// whatever its casing, because a user scanning the list reads the word, not
/// its bytes.
///
/// The padding goes first for the same reason: `sshm add --folder " ssh-config "`
/// carries the shell's spaces, and a row filed under them draws the frame's own
/// prefix just the same.
pub fn is_reserved_folder(folder: &str) -> bool {
    folder.trim().eq_ignore_ascii_case(crate::frame::SSH_CONFIG_TAG)
}

/// The refusal every writer gives for a reserved folder, in the words the user
/// sees.
///
/// One sentence because three writers ask the question — this module, the Form
/// Map, and `sshm add` — and a fourth wording would be a fourth thing to keep
/// in step with the rule.
pub fn reserved_folder_refusal(folder: &str) -> String {
    format!("folder \"{folder}\" is reserved for ssh-config rows")
}

/// Add a Connection built from `draft`, persist the set, and return it.
///
/// A folder is checked before anything is built or written: a draft naming the
/// reserved name describes a row that may not exist, and writing half of it and
/// then saying so would leave the user's file holding the impersonation.
pub fn add(config: &mut Config, draft: &ConnectionDraft) -> Result<Connection, String> {
    if is_reserved_folder(&draft.folder) {
        return Err(reserved_folder_refusal(&draft.folder));
    }

    let conn = connection_from(draft);
    config.add_connection(conn.clone());
    config.save()?;
    Ok(conn)
}

/// Edit the Connection identified by `existing_id`, persist the set, and return
/// the edited Connection.
///
/// The id is the caller's answer to "which Connection is selected" — this module
/// deliberately knows nothing about lists, filters or cursors. An id that is not
/// in the set is `Ok(None)`: nothing changed, so nothing is written, and a caller
/// never reports an edit to a Connection that does not exist.
///
/// The reserved folder is refused ahead of that lookup, because it is a fact
/// about the draft and not about the row it targets: a draft naming the reserved
/// name describes a row that may not exist whichever id it carries.
pub fn edit(
    config: &mut Config,
    existing_id: &str,
    draft: &ConnectionDraft,
) -> Result<Option<Connection>, String> {
    if is_reserved_folder(&draft.folder) {
        return Err(reserved_folder_refusal(&draft.folder));
    }

    if !config.connections.iter().any(|c| c.id == existing_id) {
        return Ok(None);
    }

    let conn = Connection {
        id: existing_id.to_string(),
        ..connection_from(draft)
    };
    config.update_connection(conn.clone());
    config.save()?;
    Ok(Some(conn))
}

/// Delete the Connection with `id`, persist the set, and return what was
/// removed. `Ok(None)` means no Connection had that id, so nothing was deleted
/// and nothing was written.
pub fn remove(config: &mut Config, id: &str) -> Result<Option<Connection>, String> {
    let removed = config.connections.iter().find(|c| c.id == id).cloned();
    if removed.is_none() {
        return Ok(None);
    }

    config.remove_connection(id);
    config.save()?;
    Ok(removed)
}

/// The draft an adopt writes from `target`, or the reason it cannot be adopted.
///
/// The copy carries the stanza's own fields and nothing else: an adopted
/// Connection lands **unfiled**, because a `Host` stanza has no folder and
/// inventing one would put the new row somewhere the user never put it. The
/// id and the provenance are not copied at all — [`connection_from`] mints a
/// fresh id and sets [`Source::Stored`], which is the whole point of an adopt:
/// what comes back is sshm's own, so it can be edited and deleted like
/// anything else in the store.
fn adopted_draft(target: &Connection) -> Result<ConnectionDraft, String> {
    if target.source != Source::SshConfig {
        return Err(format!(
            "adopt refused: {} is already in sshm's store",
            target.alias
        ));
    }
    Ok(ConnectionDraft {
        folder: String::new(),
        ..ConnectionDraft::from_connection(target)
    })
}

/// Copy the Config Host `target` into the store, persist the set, and return
/// the Connection that was written.
///
/// The only implementation of "adopt a Connection", and it is a thin one: the
/// draft is checked, then handed to [`add`], so an adopt mints its id, sets its
/// provenance and writes the file exactly the way a hand-typed add does. There
/// is no second write path for the frame to get wrong.
pub fn adopt(config: &mut Config, target: &Connection) -> Result<Connection, String> {
    let draft = adopted_draft(target)?;
    add(config, &draft)
}

/// Why a stored store refuses every adopt, in the words the frame shows.
///
/// A [`Config`] holds only rows that are already sshm's own — nothing in it
/// came from `~/.ssh/config`, so there is nothing in it to copy in. The
/// refusal is not an error the user caused; it is the shape of that store,
/// which is why it is worded about the store and not about the row.
fn nothing_to_adopt() -> String {
    "adopt refused: sshm's store holds no ssh-config rows".to_string()
}

/// The live set of Connections a frame lists, and the seam a manage action
/// persists through (#36).
///
/// The frame driver needs two things and nothing else: what to list, and a
/// way to make a delete stick. Naming them as one small trait is what lets
/// the driver hold a `&mut dyn Store` instead of reaching for `Config` and
/// the filesystem itself — and it is what keeps
/// [`remove`] as the only implementation of "delete a Connection", which
/// the module already owns.
pub trait Store {
    /// The Connections the frame lists.
    fn all(&self) -> &[Connection];

    /// Delete the Connection with `id` and persist the set.
    ///
    /// `Ok(None)` means no Connection had that id, so nothing was deleted
    /// and nothing was written — the same contract [`remove`] has.
    fn remove(&mut self, id: &str) -> Result<Option<Connection>, String>;

    /// Build the Connection the draft describes, persist the set, and
    /// return what was actually written (#37).
    ///
    /// The add half of the seam the frame's `◇ added` note rests on: the
    /// frame asks, the store answers with the Connection it made — id
    /// and all — or with an error. There is one implementation of "add a
    /// Connection" in this crate and the frame does not get to write a
    /// second one.
    fn add(&mut self, draft: &ConnectionDraft) -> Result<Connection, String>;

    /// Replace the Connection with `existing_id` with the one `draft`
    /// describes, persist the set, and return what was actually written
    /// (#37).
    ///
    /// The edit half of the seam the frame's `◇ edited` note rests on.
    /// The id is carried across rather than minted: an edit answers about
    /// *this* Connection, and a new id would orphan every reference to the
    /// old one. `Ok(None)` means no Connection had that id, so nothing
    /// was changed and nothing was written — the same contract [`edit`]
    /// has, and the reason the frame must not say `edited` until this
    /// comes back `Some`.
    fn update(
        &mut self,
        existing_id: &str,
        draft: &ConnectionDraft,
    ) -> Result<Option<Connection>, String>;

    /// Copy the Config Host `target` into the set, persist it, and return the
    /// Connection that was written.
    ///
    /// The adopt half of the seam the frame's `◇ adopted` note rests on. A
    /// Config Host is the user's own stanza — sshm reads `~/.ssh/config` and
    /// never writes it — so the one thing a frame may do with one is offer a
    /// copy of it into sshm's store, where it can be edited and deleted like
    /// anything else.
    ///
    /// `Err` unless `target` really is a config row: a Connection already in
    /// the store has nothing to adopt, and a store that wrote a second copy of
    /// it would be duplicating a row the user owns and hiding the copy behind
    /// it under the collision rule.
    fn adopt(&mut self, target: &Connection) -> Result<Connection, String>;
}

impl Store for Config {
    fn all(&self) -> &[Connection] {
        &self.connections
    }

    fn remove(&mut self, id: &str) -> Result<Option<Connection>, String> {
        remove(self, id)
    }

    fn add(&mut self, draft: &ConnectionDraft) -> Result<Connection, String> {
        add(self, draft)
    }

    fn update(
        &mut self,
        existing_id: &str,
        draft: &ConnectionDraft,
    ) -> Result<Option<Connection>, String> {
        edit(self, existing_id, draft)
    }

    /// Always a refusal: a stored store holds only rows that are already
    /// sshm's own, so there is nothing in it to copy in.
    fn adopt(&mut self, _target: &Connection) -> Result<Connection, String> {
        Err(nothing_to_adopt())
    }
}

/// A set that is real for the frame and persisted by nobody.
///
/// The visual harness needs deletes to actually remove rows so the frame can
/// be looked at doing it, without that delete reaching the user's
/// `connections.json`. The removal is not a fake: the Connection really is
/// gone from the set the frame is showing. Only the durability is absent.
#[derive(Debug, Clone, Default)]
pub struct Ephemeral {
    connections: Vec<Connection>,
}

impl Ephemeral {
    /// An ephemeral set seeded from `connections`.
    pub fn new(connections: Vec<Connection>) -> Self {
        Self { connections }
    }
}

impl Store for Ephemeral {
    fn all(&self) -> &[Connection] {
        &self.connections
    }

    fn remove(&mut self, id: &str) -> Result<Option<Connection>, String> {
        let position = self.connections.iter().position(|c| c.id == id);
        Ok(position.map(|i| self.connections.remove(i)))
    }

    fn add(&mut self, draft: &ConnectionDraft) -> Result<Connection, String> {
        // The same refusal as the persisted store, for the same reason: a row
        // filed under the frame's own word draws `[ssh-config]` without being
        // one, and a harness that admitted it would be showing the user a row
        // the real store would have refused.
        if is_reserved_folder(&draft.folder) {
            return Err(reserved_folder_refusal(&draft.folder));
        }

        // Built through the same `connection_from` the persisted path
        // uses, so the row the harness shows is the row the real store
        // would have written. Only the durability is absent.
        let conn = connection_from(draft);
        self.connections.push(conn.clone());
        Ok(conn)
    }

    fn update(
        &mut self,
        existing_id: &str,
        draft: &ConnectionDraft,
    ) -> Result<Option<Connection>, String> {
        // Refused ahead of the lookup, as the persisted path does: the folder is
        // a fact about the draft, not about the row it targets.
        if is_reserved_folder(&draft.folder) {
            return Err(reserved_folder_refusal(&draft.folder));
        }

        // Same discipline as `add`: built through `connection_from`, with
        // the *existing* id kept. A new id here would make the harness's
        // edited row a different Connection from the one the real store
        // would have changed.
        let position = self.connections.iter().position(|c| c.id == existing_id);
        let Some(i) = position else {
            return Ok(None);
        };
        let conn = Connection {
            id: existing_id.to_string(),
            ..connection_from(draft)
        };
        self.connections[i] = conn.clone();
        Ok(Some(conn))
    }

    fn adopt(&mut self, target: &Connection) -> Result<Connection, String> {
        // The same rule and the same construction as the persisted path, with
        // only the durability absent: the visual harness has to be able to show
        // an adopt, and a harness that admitted a row the real store refuses
        // would be demoing a write that cannot happen.
        let draft = adopted_draft(target)?;
        self.add(&draft)
    }
}
