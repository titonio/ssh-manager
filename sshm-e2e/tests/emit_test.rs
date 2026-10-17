//! The emit axis: what the picker's answer is *for*.
//!
//! Every other file in this suite watches the screen. The screen is the least
//! interesting half of an emit — the user's whole point is the command that
//! leaves, or the text that lands on their shell's command line. A frame that
//! looks perfect and then runs the wrong `ssh`, or prints its own chrome into a
//! captured pipe, is a broken tool that happens to be pretty.
//!
//! So these tests assert on the two things a user actually receives: the argv
//! handed to `ssh`, recorded by a stub `ssh` found on `PATH` — the real
//! `Command::new("ssh")` path runs untouched, it just finds our stub — and the
//! bytes on a stdout that has been captured the way `$(sshm pick)` captures it,
//! which is the only way to prove the frame's own control sequences stayed off
//! the pipe.

use std::path::Path;

use sshm_e2e::fixture::{connection, connection_full, sshm_binary};
use sshm_e2e::{Fixture, Pty, DEFAULT_TIMEOUT};

/// Selecting a Connection runs `ssh` with exactly that Connection's fields.
///
/// The argv is the contract, and it is built from four fields with two
/// conditionals in it: the key only when there is one, the port only when it is
/// not the default. Asserting the whole argv as a literal is the point — a test
/// that re-derived the expected argv from the same rules would pass whatever
/// those rules said.
#[test]
fn selecting_a_connection_runs_ssh_with_exactly_the_connection_s_fields() {
    let fixture = Fixture::new()
        .with_connections(&[connection_full(
            "build-01",
            "10.0.0.42",
            "ci",
            2222,
            "/keys/id_build",
        )])
        .and_then(|f| f.with_ssh_shim())
        .expect("seed");
    let mut pty = Pty::spawn(sshm_binary(), &[], &fixture).expect("spawn sshm");
    pty.wait_for_text("build-01").expect("the picker draws");
    pty.key(b"\r");

    pty.wait_until_pumping("ssh to be invoked and recorded", DEFAULT_TIMEOUT, || {
        fixture.ssh_calls_are_complete()
    })
    .expect("ssh was invoked");
    let recorded = fixture.ssh_calls().expect("the log reads");
    assert_eq!(
        recorded.len(),
        1,
        "exactly one ssh invocation, got {recorded:?}\n{}",
        pty.grid()
    );
    assert_eq!(
        recorded[0],
        [
            "-i".to_string(),
            "/keys/id_build".to_string(),
            "-p".to_string(),
            "2222".to_string(),
            "ci@10.0.0.42".to_string(),
        ],
        "the argv handed to ssh is the Connection's fields, in the order ssh expects"
    );

    pty.wait_exit(DEFAULT_TIMEOUT)
        .expect("sshm leaves with ssh's status")
        .assert_code(0);

    // The handover to a real `ssh` is a third way out of a frame, and it owes
    // the terminal back cooked like the other two: whatever that command was,
    // the user's shell is the next thing reading this terminal.
    assert!(
        pty.termios_is_cooked().expect("termios of the pty"),
        "handing over to ssh left the terminal out of cooked mode"
    );
}

/// A `[ssh-config]` row runs the bare alias, because the alias is the name the
/// user's own file answers to.
///
/// The mirror of the first test, and the reason it is not enough on its own: a
/// Config Host has the same four fields as a stored Connection, so a build path
/// that treated every row the same would re-encode them into
/// `ssh -p 2222 ci@build.example.com` and still reach the machine in the easy
/// case — while silently ignoring whatever the `Host` line is really doing, a
/// `ProxyJump`, a `Match` block, an alias that is not a hostname at all. The
/// stanza carries a `User` and a non-default `Port` so that wrong shape is
/// visible in the log instead of merely possible.
#[test]
fn selecting_a_config_host_runs_the_bare_alias_and_nothing_else() {
    let fixture = Fixture::new()
        .with_ssh_config("Host build-box\n  HostName build.example.com\n  User ci\n  Port 2222\n")
        .and_then(|f| f.with_ssh_shim())
        .expect("seed");
    let mut pty = Pty::spawn(sshm_binary(), &[], &fixture).expect("spawn sshm");
    pty.wait_for_text("build-box")
        .expect("the picker draws the config row");
    pty.key(b"\r");

    pty.wait_until_pumping("ssh to be invoked and recorded", DEFAULT_TIMEOUT, || {
        fixture.ssh_calls_are_complete()
    })
    .expect("ssh was invoked");
    let recorded = fixture.ssh_calls().expect("the log reads");
    assert_eq!(
        recorded,
        [vec!["build-box".to_string()]],
        "a config host is the alias and nothing else — no `-p`, no `-i`, no `user@`"
    );

    pty.wait_exit(DEFAULT_TIMEOUT)
        .expect("sshm leaves with ssh's status")
        .assert_code(0);
}

/// Escape from bare `sshm` runs nothing at all.
///
/// The mirror image of the test above, and the half that matters more: a cancel
/// that still exec'd would open a connection the user just said no to. The stub
/// is on `PATH` for exactly this — an empty log is the proof, where the screen
/// could only ever show an absence.
#[test]
fn escape_from_bare_sshm_executes_nothing() {
    let fixture = Fixture::new()
        .with_connections(&[connection_full(
            "build-01",
            "10.0.0.42",
            "ci",
            2222,
            "/keys/id_build",
        )])
        .and_then(|f| f.with_ssh_shim())
        .expect("seed");

    let mut pty = Pty::spawn(sshm_binary(), &[], &fixture).expect("spawn sshm");
    pty.wait_for_text("build-01").expect("the picker draws");

    pty.key(b"\x1b");
    pty.wait_exit(DEFAULT_TIMEOUT)
        .expect("the frame closes")
        .assert_code(130);

    assert!(
        !fixture.ssh_calls_are_complete(),
        "a cancelled picker ran ssh anyway: {:?}",
        fixture.ssh_calls().expect("the log reads")
    );
    assert!(
        fixture.ssh_calls().expect("the log reads").is_empty(),
        "nothing was executed on the way out"
    );
}

/// A Connection on the default port with no key sends only the target.
///
/// The other half of the two conditionals: nothing that is not set should
/// arrive, because `ssh -p 22` is harmless but `ssh -i ''` is an error, and a
/// stub that recorded `-p 22` would let a mutant that always emits the port
/// through.
#[test]
fn a_connection_on_the_default_port_with_no_key_sends_only_the_target() {
    let fixture = Fixture::new()
        .with_connections(&[connection("web-01", "10.0.0.7", "deploy")])
        .and_then(|f| f.with_ssh_shim())
        .expect("seed");
    let mut pty = Pty::spawn(sshm_binary(), &[], &fixture).expect("spawn sshm");
    pty.wait_for_text("web-01").expect("the picker draws");
    pty.key(b"\r");

    pty.wait_until_pumping("ssh to be invoked and recorded", DEFAULT_TIMEOUT, || {
        fixture.ssh_calls_are_complete()
    })
    .expect("ssh was invoked");
    let recorded = fixture.ssh_calls().expect("the log reads");
    assert_eq!(
        recorded,
        [vec!["deploy@10.0.0.7".to_string()]],
        "only the target, with no flags for fields that were never set"
    );
}

/// The picker prints the alias and nothing else into a captured pipe.
///
/// This is the whole shell integration in one line: `alias=$(sshm pick)`. The
/// frame cannot be on that pipe — not one escape sequence — or the assignment
/// captures the frame instead of the alias. So the capture is driven from a
/// real shell, in a real terminal, and the shell itself reports what it caught.
/// Anything of the frame's that leaked onto stdout would be inside the brackets.
#[test]
fn the_picker_prints_the_alias_and_nothing_else_into_a_captured_pipe() {
    let fixture = Fixture::new()
        .with_connections(&[
            connection("web-01", "10.0.0.7", "deploy"),
            connection("db-01", "10.0.0.8", "root"),
        ])
        .expect("seed");

    let mut pty = spawn_capture(&fixture, "pick");
    pty.wait_for_text("web-01").expect("the picker draws");
    // Narrow with a keystroke rather than pick the first row, so the test
    // proves the *selection* travelled and not merely the first row.
    pty.key(b"db");
    pty.wait_for_text("db-01").expect("the query filters");
    pty.key(b"\r");

    pty.wait_for_text("GOT[db-01]")
        .expect("the shell captured exactly the alias");
}

/// Escape in a captured picker prints nothing and leaves the cancel status.
///
/// The cut-over contract for a shell binding: cancelling must not put a stray
/// empty string into the variable the user was filling in, and must say "I
/// cancelled" in the one channel that survives a capture — the exit status.
#[test]
fn escape_in_a_captured_picker_prints_nothing_and_leaves_the_cancel_status() {
    let fixture = Fixture::new()
        .with_connections(&[connection("web-01", "10.0.0.7", "deploy")])
        .expect("seed");

    let mut pty = spawn_capture_with_status(&fixture, "pick");
    pty.wait_for_text("web-01").expect("the picker draws");
    pty.key(b"\x1b");

    pty.wait_for_text("GOT[]:130")
        .expect("nothing was printed and the status says cancelled");
}

/// A seeded query opens the picker already filtered.
///
/// The Shell Widget's `**<TAB>` path hands a half-typed word to `--query`, so
/// the frame has to arrive with that query applied: the matching Connection
/// visible, the non-matching one gone. Then the selection of the row that
/// survived is what the pipe receives.
#[test]
fn a_seeded_query_opens_the_picker_already_filtered() {
    let fixture = Fixture::new()
        .with_connections(&[
            connection("web-01", "10.0.0.7", "deploy"),
            connection("db-01", "10.0.0.8", "root"),
        ])
        .expect("seed");

    let mut pty = spawn_capture(&fixture, "pick --query db");
    pty.wait_for_frame().expect("the match is drawn");
    assert!(
        !pty.grid().contains("web-01"),
        "the seeded query left the non-matching Connection on the grid:\n{}",
        pty.grid()
    );
    pty.key(b"\r");

    pty.wait_for_text("GOT[db-01]")
        .expect("the surviving row is what the shell captured");
}

/// Run `sshm <command>` inside a shell whose stdout is captured by `$( )`, and
/// have the shell print what it caught.
///
/// The frame still has a terminal to draw on — the pty, reached through
/// `/dev/tty` — while stdout is a pipe, which is precisely the arrangement the
/// shell integration depends on and the one no in-crate test can stage.
fn spawn_capture(fixture: &Fixture, command: &str) -> Pty {
    let script = format!(
        "out=$('{}' {command}); printf 'GOT[%s]\\n' \"$out\"",
        sshm_binary().display()
    );
    Pty::spawn(Path::new("/bin/sh"), &["-c", &script], fixture).expect("the shell runs")
}

/// The same, reporting the exit status beside the captured text.
fn spawn_capture_with_status(fixture: &Fixture, command: &str) -> Pty {
    let script = format!(
        "out=$('{}' {command}); status=$?; printf 'GOT[%s]:%s\\n' \"$out\" \"$status\"",
        sshm_binary().display()
    );
    Pty::spawn(Path::new("/bin/sh"), &["-c", &script], fixture).expect("the shell runs")
}
