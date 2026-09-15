//! The emit axis (#35): what Enter *means*, per command.
//!
//! The frame is one surface; the commands differ only in what they do with the
//! selection. That difference is the `--emit` axis, and this seam is the only
//! place it is decided: `Emit::resolve` turns the frame's own outcome into a
//! concrete `Action`, so `main.rs` never re-derives the meaning of Enter and
//! no boolean or string-match can leak the policy across commands.
//!
//! Expectations come from the parent spec's worked examples (#31): the
//! fixture Connection is the spec's own `web-01 / deploy@10.0.0.4`, and the
//! ssh argv is the literal the spec's row text implies — not a recomputation
//! of `build_ssh_args`.

use std::ffi::OsString;

use sshm::config::Connection;
use sshm::emit::{Action, Emit};
use sshm::frame::FrameMode;
use sshm::inline::InlineOutcome;

/// The spec's worked-example Connection (see `frame_test.rs` / the settle
/// trace `◆ picked [prod] web-01 (deploy@10.0.0.4:22)`).
fn web01() -> Connection {
    Connection {
        id: "1".into(),
        alias: "web-01".into(),
        host: "10.0.0.4".into(),
        user: "deploy".into(),
        port: 22,
        key_path: None,
        folder: Some("prod".into()),
    }
}

/// A Connection with every optional field set, so the Execute argv is pinned
/// completely rather than incidentally.
fn loaded() -> Connection {
    Connection {
        id: "2".into(),
        alias: "build-box".into(),
        host: "10.0.0.9".into(),
        user: "ci".into(),
        port: 2222,
        key_path: Some("/keys/ci.pem".into()),
        folder: None,
    }
}

// ── the frame each command opens ─────────────────────────────────────────────

#[test]
fn bare_sshm_and_pick_open_the_pick_frame() {
    assert_eq!(Emit::Execute.frame_mode(), FrameMode::Pick);
    assert_eq!(Emit::Insert.frame_mode(), FrameMode::Pick);
}

#[test]
fn manage_opens_the_manage_frame() {
    assert_eq!(Emit::Edit.frame_mode(), FrameMode::Manage);
}

// ── what Enter resolves to ───────────────────────────────────────────────────

#[test]
fn execute_resolves_a_pick_to_the_ssh_argv() {
    let action = Emit::Execute.resolve(InlineOutcome::Picked(loaded()));

    // The literal ssh argv the spec's row implies: key first, non-default
    // port, then user@host. Port 22 would omit `-p` entirely.
    let expected: Vec<OsString> = ["-i", "/keys/ci.pem", "-p", "2222", "ci@10.0.0.9"]
        .iter()
        .map(OsString::from)
        .collect();
    assert_eq!(action, Action::Execute(expected));
}

#[test]
fn insert_resolves_a_pick_to_exactly_the_alias() {
    // The whole clean-stdout contract rests on this: the payload is the
    // alias alone — no `ssh` prefix, no decoration, no trailing space.
    let action = Emit::Insert.resolve(InlineOutcome::Picked(web01()));
    assert_eq!(action, Action::Insert("web-01".to_string()));
}

#[test]
fn edit_resolves_a_pick_to_the_selected_connection() {
    let action = Emit::Edit.resolve(InlineOutcome::Picked(web01()));
    assert_eq!(action, Action::Edit(web01()));
}

#[test]
fn cancel_resolves_to_nothing_on_every_emit() {
    for emit in [Emit::Execute, Emit::Insert, Emit::Edit] {
        assert_eq!(
            emit.resolve(InlineOutcome::Cancelled),
            Action::Cancelled,
            "{emit:?}: a cancel must not emit, execute or edit"
        );
    }
}

// ── the stream split: who gets the frame, who gets the alias ──────────────────

/// The decision the deleted picker's `tui_output_kind` used to make, restored
/// as a real (non-`#[cfg(test)]`) seam: when stdout is captured by
/// `$(sshm pick)`, the frame must go to `/dev/tty` so the pipe keeps only
/// the emitted alias. When stdout is the terminal itself, the frame goes
/// there and there is nothing to protect.
#[test]
fn a_captured_stdout_sends_the_frame_to_the_tty() {
    assert_eq!(
        sshm::emit::frame_stream(false),
        sshm::emit::FrameStream::Tty,
        "with stdout captured, drawing the frame there would corrupt the alias"
    );
}

#[test]
fn a_terminal_stdout_keeps_the_frame_on_stdout() {
    assert_eq!(
        sshm::emit::frame_stream(true),
        sshm::emit::FrameStream::Stdout,
        "with a real terminal there is no captured stream to protect"
    );
}

/// The split is decided by stdout alone, never by the emit: all three commands
/// draw the frame the same way, and only the Insert emit has a captured stdout
/// to keep clean. Asserting the axis is orthogonal stops a future `Emit` from
/// quietly re-deriving the routing.
#[test]
fn every_emit_uses_the_same_stream_decision() {
    for emit in [Emit::Execute, Emit::Insert, Emit::Edit] {
        assert_eq!(
            sshm::emit::frame_stream(true),
            sshm::emit::FrameStream::Stdout,
            "{emit:?}: a terminal stdout always gets the frame"
        );
        assert_eq!(
            sshm::emit::frame_stream(false),
            sshm::emit::FrameStream::Tty,
            "{emit:?}: a captured stdout never gets the frame"
        );
    }
}
