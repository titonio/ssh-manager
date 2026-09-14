//! The Connection manager.
//!
//! Every change to the Connection set lives here: add, edit, delete, the
//! `~/.ssh/config` import, and the update-note read. Each operation mutates the
//! set, persists it, and reports what came back, so a surface can render its own
//! trace without re-implementing the rules.
//!
//! This is business logic only. Nothing here knows how a Connection is drawn,
//! filtered or selected — a caller passes the draft it collected and the id it
//! is acting on, and gets the changed Connection out. That is what lets the
//! fullscreen TUI and the inline frames share one implementation.

use crate::config::{Config, Connection};

/// The field values a Connection is built from while the user adds or edits one.
///
/// Every field is a string because it arrives from keystroke-collected input, not
/// from a parsed config file: the port is digits-as-text, and an optional field
/// the user left alone is empty rather than absent.
#[derive(Debug, Clone, Default)]
pub struct ConnectionDraft {
    pub alias: String,
    pub host: String,
    pub user: String,
    pub port: String,
    pub key_path: String,
    pub folder: String,
}

impl ConnectionDraft {
    /// Reset to a blank draft, with the SSH default port already filled in.
    pub fn clear(&mut self) {
        self.alias.clear();
        self.host.clear();
        self.user.clear();
        self.port = "22".to_string();
        self.key_path.clear();
        self.folder.clear();
    }

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
fn connection_from(draft: &ConnectionDraft) -> Connection {
    Connection {
        id: uuid::Uuid::new_v4().to_string(),
        alias: draft.alias.clone(),
        host: draft.host.clone(),
        user: draft.user.clone(),
        port: draft.port.parse().unwrap_or(22),
        key_path: none_if_blank(&draft.key_path),
        folder: none_if_blank(&draft.folder),
    }
}

fn none_if_blank(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

/// Add a Connection built from `draft`, persist the set, and return it.
pub fn add(config: &mut Config, draft: &ConnectionDraft) -> Result<Connection, String> {
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
/// in the set changes nothing, which is how the store has always treated one.
pub fn edit(
    config: &mut Config,
    existing_id: &str,
    draft: &ConnectionDraft,
) -> Result<Connection, String> {
    let conn = Connection {
        id: existing_id.to_string(),
        ..connection_from(draft)
    };
    config.update_connection(conn.clone());
    config.save()?;
    Ok(conn)
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

/// Import Connections from the user's `~/.ssh/config`, persist the set, and
/// return how many Connections were added. Entries already in the set are left
/// alone, so importing twice adds nothing the second time.
pub fn import(config: &mut Config) -> Result<usize, String> {
    let imported = crate::config::import_from_ssh_config(config);
    config.save()?;
    Ok(imported)
}

/// What an update check leaves a surface to show.
///
/// The check itself belongs to `update.rs`; this is only the shape both the
/// fullscreen TUI and the inline frames want: a note carrying versions, nothing
/// at all, or the reason the check failed.
#[derive(Debug, Clone)]
pub enum UpdateNote {
    /// A newer release exists, with the versions to name in the note.
    Available(crate::update::UpdateInfo),
    /// Nothing to show.
    None,
    /// The check failed; the string is what the surface shows.
    Failed(String),
}

/// Read the update note a frame shows above itself.
///
/// Inside a source checkout this returns `UpdateNote::None` without going near
/// the network: running the tool from a checkout should not block a first paint
/// on GitHub, nor nag about a version that is already local.
pub fn read_update_note() -> UpdateNote {
    if std::env::var("CARGO_MANIFEST_DIR").is_ok() {
        return UpdateNote::None;
    }

    update_note_from(crate::update::check_for_update())
}

/// Turn the outcome of an update check into the note a surface shows.
pub fn update_note_from(result: crate::update::UpdateResult) -> UpdateNote {
    match result {
        crate::update::UpdateResult::UpdateAvailable { version } => {
            UpdateNote::Available(crate::update::UpdateInfo {
                current_version: env!("CARGO_PKG_VERSION").to_string(),
                new_version: version,
            })
        }
        crate::update::UpdateResult::NoUpdate => UpdateNote::None,
        crate::update::UpdateResult::Error(message) => UpdateNote::Failed(message),
    }
}
