//! The live view of the Connection set: sshm's store plus whatever
//! `~/.ssh/config` holds at the moment the frame opened.
//!
//! The frame does not know this module exists. It is handed `&mut dyn Store` and
//! asks for rows, exactly as it always did — which is the point of putting the
//! merge here rather than in the frame: one seam, one implementation of "what is
//! this frame listing", and the drawing code keeps taking `&[Connection]`.
//!
//! # The freshness contract
//!
//! **The ssh config is read when the store is built and again after every
//! mutation, and never in between.**
//!
//! Reading at construction is enough for freshness, because a frame is a
//! short-lived process: it opens, the user answers it, it closes. By the time
//! the file could have changed, this store is gone. Re-reading on every [`Store::all`]
//! would be worse than useless — the list would change under the cursor between
//! two keystrokes, and a cursor is only meaningful against a list that is not
//! moving. So the merged list is frozen while a frame is open.
//!
//! A mutation is the exception, and the reason it is one: add, edit and delete
//! change the stored half of the list, and the collision rule means they change
//! which config rows survive too. Adopt a stanza and its config row has to
//! disappear in the same keystroke that wrote the store row, or the frame is
//! showing a duplicate the user just resolved. So every mutation goes to the
//! inner [`Config`], persists there through the one implementation
//! [`crate::connections`] owns, and then re-reads and re-merges.
//!
//! Nothing here writes `~/.ssh/config`, and there is no file watcher: the config
//! is the user's file, edited in their editor, and sshm only ever reads it.
//! A file that cannot be read contributes no rows — an absent `~/.ssh/config`
//! is an ordinary machine, not an error.

use std::path::PathBuf;

use crate::config::{self, Config, Connection};
use crate::connections::{self, ConnectionDraft, Store};

/// The `Store` a frame is handed: sshm's own Connections with the usable
/// `Host` stanzas of `~/.ssh/config` merged into the same flat list.
pub struct LiveStore {
    /// The persisted half, and the only thing a mutation writes through.
    config: Config,
    /// Where to read the config from; `None` when no home directory resolved,
    /// which leaves the store on its own.
    ssh_config_path: Option<PathBuf>,
    /// The merged list: stored rows plus the config rows that survive the
    /// collision rule, in the order a frame should draw them.
    ///
    /// A field rather than a call, because [`Store::all`] answers with a slice
    /// and so the merge has to have happened before anyone asks — which is
    /// exactly what the freshness contract promises it has.
    merged: Vec<Connection>,
}

impl LiveStore {
    /// Wrap `config` and merge in the ssh config at `ssh_config_path`, right
    /// away: a frame must never paint a list that predates its own opening.
    pub fn new(config: Config, ssh_config_path: Option<PathBuf>) -> Self {
        let mut store = Self {
            config,
            ssh_config_path,
            merged: Vec::new(),
        };
        store.refresh();
        store
    }

    /// Re-read the ssh config and rebuild the merged list from it.
    ///
    /// Called at construction and after every mutation, and by nothing else:
    /// that is the whole freshness contract, and it is one function so there is
    /// only one place to look for when the list was last computed. A path that
    /// is absent or unreadable parses as no entries, so a mutation never fails
    /// because of the state of someone else's file.
    fn refresh(&mut self) {
        let entries = match &self.ssh_config_path {
            Some(path) => config::parse_ssh_config(&path.to_string_lossy()),
            None => Vec::new(),
        };
        self.merged = config::merge_ssh_config(&self.config.connections, &entries);
    }
}

impl Store for LiveStore {
    /// The merged list, frozen since the last read: the stored rows and the
    /// config rows that survived the collision rule, in the order to draw them.
    fn all(&self) -> &[Connection] {
        &self.merged
    }

    fn remove(&mut self, id: &str) -> Result<Option<Connection>, String> {
        let removed = connections::remove(&mut self.config, id)?;
        self.refresh();
        Ok(removed)
    }

    fn add(&mut self, draft: &ConnectionDraft) -> Result<Connection, String> {
        let added = connections::add(&mut self.config, draft)?;
        self.refresh();
        Ok(added)
    }

    fn update(
        &mut self,
        existing_id: &str,
        draft: &ConnectionDraft,
    ) -> Result<Option<Connection>, String> {
        let edited = connections::edit(&mut self.config, existing_id, draft)?;
        self.refresh();
        Ok(edited)
    }

    /// Copy a Config Host into the store and re-merge, which is the whole
    /// reason adopt needs the freshness contract at all.
    ///
    /// The copy matches the stanza it was made from on host and user, so the
    /// collision rule retires the config row the moment the store row exists:
    /// without the re-merge the frame would go on listing a row the user has
    /// just resolved, and the `◇ adopted` note would be describing a duplicate.
    fn adopt(&mut self, target: &Connection) -> Result<Connection, String> {
        let adopted = connections::adopt(&mut self.config, target)?;
        self.refresh();
        Ok(adopted)
    }
}
