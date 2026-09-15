//! The emit axis: what Enter *means*, per command.
//!
//! Bare `sshm`, `sshm pick` and `sshm manage` all draw the same inline frame
//! ([`crate::frame`]) and drive it with the same keys
//! ([`crate::inline`]). They differ in exactly one thing: what happens to the
//! selected Connection when the user presses Enter. That difference is the
//! `--emit` axis from the parent spec (#31), and this module is the only
//! place it is modelled.
//!
//! The point of making it a type instead of a boolean or a `match` on the
//! command name in `main.rs` is locality: the whole policy — which frame each
//! command opens, and what a pick or a cancel resolves to — reads in one
//! file, and a new command cannot accidentally re-derive Enter's meaning.
//!
//! [`Emit::resolve`] is pure. The effects (executing `ssh`, writing the
//! emitted alias to stdout, handing the Connection to the edit path) belong
//! to the runner, which just matches on the [`Action`] it is given.

use crate::config::Connection;
use crate::frame::FrameMode;
use crate::inline::InlineOutcome;
use crate::ssh::build_ssh_args;
use std::ffi::OsString;

/// What the command was asked to do with the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emit {
    /// Bare `sshm`: Enter executes `ssh` against the Connection.
    Execute,
    /// `sshm pick`: Enter emits the alias on stdout, for the shell to insert.
    Insert,
    /// `sshm manage`: Enter hands the Connection to the edit path.
    Edit,
}

/// What the runner must now do. The variant is the whole decision; the
/// payload is everything the effect needs and nothing more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Run `ssh` with these arguments.
    Execute(Vec<OsString>),
    /// Write this line to stdout — the alias alone, so `$(sshm pick)`
    /// captures nothing but the selection.
    Insert(String),
    /// The selected Connection, for the edit path.
    Edit(Connection),
    /// The user cancelled: emit nothing, execute nothing.
    Cancelled,
}

impl Emit {
    /// The frame this command opens. Pick and bare share the pick frame;
    /// manage wears the manage header. The frame grammar is identical — only
    /// the header and the hint rail differ, which is the whole point of
    /// sharing one seam.
    pub fn frame_mode(self) -> FrameMode {
        match self {
            Emit::Execute | Emit::Insert => FrameMode::Pick,
            Emit::Edit => FrameMode::Manage,
        }
    }

    /// Turn the frame's outcome into the action this emit means.
    ///
    /// A cancel resolves to `Cancelled` under every emit: leaving the frame
    /// never executes, inserts or edits, whatever command opened it.
    pub fn resolve(self, outcome: InlineOutcome) -> Action {
        match (self, outcome) {
            (_, InlineOutcome::Cancelled) => Action::Cancelled,
            (Emit::Execute, InlineOutcome::Picked(conn)) => Action::Execute(build_ssh_args(&conn)),
            (Emit::Insert, InlineOutcome::Picked(conn)) => Action::Insert(conn.alias),
            (Emit::Edit, InlineOutcome::Picked(conn)) => Action::Edit(conn),
        }
    }
}
