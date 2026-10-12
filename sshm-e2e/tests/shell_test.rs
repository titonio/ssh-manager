//! The Shell Widget in real shells: zsh and bash, driven by their own bindings.
//!
//! Everything else in this suite launches `sshm` directly. This file launches a
//! *shell* and asks it to run `sshm`, which is the only way to test the part that
//! is not written in Rust: the ZLE widget, the readline function, the `bindkey`
//! and `bind -x` lines, and the assumption that a keystroke the user presses with
//! the left hand arrives intact. Every one of those is shell code, and shell code
//! that is only ever read by a human is shell code that has never been run.
//!
//! Both shells start with nothing from the machine: no rc files, a prompt the
//! test sets itself, and `sshm` on `PATH` from the build. The init script is
//! sourced the way the README says to source it, because the script *is* the
//! product here — copying its text into the test would freeze a copy that could
//! drift the moment the real one changed. What the tests assert is what the user
//! sees on the prompt line, which is the only surface any of this machinery has.

use std::time::Duration;

use sshm_e2e::fixture::{seeded_fixture, sshm_binary, Fixture};
use sshm_e2e::{Pty, DEFAULT_TIMEOUT};

/// The exact bytes the Shell Widget's binding waits for.
///
/// Ctrl+Alt+S does not arrive as one byte. The Alt half arrives as a leading
/// `ESC`, and the S half as 0x13 — Device Control Three, what Ctrl+S produces —
/// so the chord on the wire is `ESC` `0x13`. That is what `bindkey '\e^S'` in
/// the zsh script and `bind -x '"\e\C-s"'` in the bash script are both matching,
/// and it is pinned here rather than derived from the scripts: the harness has to
/// send what a keyboard sends, and if the binding ever changes, changing this
/// constant is a deliberate act rather than a silent one.
///
/// 0x13 is also XOFF. With `IXON` on, the terminal driver eats it and stops
/// output instead of delivering it, so the chord cannot be sent at all until
/// `IXON` is off. The harness clears it on the slave when the shell is spawned,
/// and the setup line below runs `stty -ixon` as well: a shell saves the
/// terminal's settings when it starts and puts them back after every line, so a
/// clear that lands before that save is quietly undone by the first restore.
/// Telling the shell itself is both the durable fix and what a real user's rc
/// file has to do for this binding to be reachable.
const BIND: &[u8] = b"\x1b\x13";

/// The prompt every test sets, so a screen can be read as "what is on the buffer".
const PROMPT: &str = "e2e# ";

/// One shell, and how to get it into a state the tests can drive.
struct Shell {
    name: &'static str,
    program: &'static str,
    args: &'static [&'static str],
    /// Turns flow control off, sets the prompt and sources the widget, as one
    /// line of shell.
    setup: &'static str,
}

const ZSH: Shell = Shell {
    name: "zsh",
    program: "zsh",
    // `-d` so the shell does not move out of the fixture's home, `-f` so no
    // `.zshrc` from the machine running the tests is part of the result.
    args: &["-d", "-f"],
    setup: "stty -ixon; PROMPT='e2e# '; eval \"$(sshm init zsh)\"",
};

const BASH: Shell = Shell {
    name: "bash",
    program: "bash",
    // `--norc` and `--noprofile` for the same reason `-f` is there: the only
    // thing in this shell is what the test put in it.
    args: &["--norc", "--noprofile"],
    setup: "stty -ixon; PS1='e2e# '; eval \"$(sshm init bash)\"",
};

/// A home with two Connections and `sshm` itself on `PATH`.
fn shell_fixture() -> Fixture {
    let binary = sshm_binary();
    let dir = binary.parent().expect("the binary is in a directory");
    let inherited = std::env::var("PATH").unwrap_or_default();
    seeded_fixture(&["web-01", "db-01"])
        .with_env("PATH", format!("{}:{}", dir.display(), inherited))
}

/// Type `text`, press the binding, and wait for the Inline Picker to be up.
fn open_picker(shell: &Shell, text: &str) -> (Fixture, Pty) {
    let fixture = shell_fixture();
    let mut pty = Pty::spawn(shell.program, shell.args, &fixture)
        .unwrap_or_else(|error| panic!("spawn {}: {error}", shell.name));
    pty.clear_ixon()
        .unwrap_or_else(|error| panic!("clear IXON for {}: {error}", shell.name));
    pty.send(format!("{}\necho \"ok-$((1+1))\"\n", shell.setup).as_bytes())
        .expect("the shell takes the setup line");
    pty.wait_for_text("ok-2")
        .unwrap_or_else(|_| panic!("{} sourced its widget", shell.name));

    pty.send(text.as_bytes()).expect("the shell takes typing");
    // Let the shell show the typing before the chord follows it. The chord is
    // two bytes, `ESC` then `0x13`, and a line editor only knows an Escape is
    // the start of its own binding once it has read the byte behind it; a
    // chord sent in the same breath as the text it follows can arrive as one
    // unread lump and be parsed as whatever the editor's timeout decides. A
    // hand gives a chord a moment of its own, and the moment here is the echo
    // — content, not a clock.
    let echoed = format!("{PROMPT}{text}");
    pty.wait_until(
        &format!("{} to echo {text:?}", shell.name),
        DEFAULT_TIMEOUT,
        move |screen| {
            screen
                .contents()
                .lines()
                .any(|line| line.trim_end() == echoed)
        },
    )
    .unwrap_or_else(|_| panic!("{} echoes what it was typed", shell.name));

    pty.key(BIND);
    (fixture, pty)
}

/// Wait for the line the shell is offering to run, and hand it back.
fn wait_for_buffer(pty: &mut Pty, text: &str) {
    let want = format!("{PROMPT}{text}");
    pty.wait_until(
        &format!("the buffer holding {text:?}"),
        DEFAULT_TIMEOUT,
        move |screen| {
            screen
                .contents()
                .lines()
                .any(|line| line.trim_end() == want)
        },
    )
    .unwrap_or_else(|error| panic!("{error}"));
}

/// The chord's whole handover, given more room than any other wait here.
///
/// Every other wait in the suite is answered by a process that is already
/// running. This one is answered by a process that has to be created: the
/// binding fires, the shell forks, the binary is loaded, and only then does
/// anything reach the terminal. On a machine running several copies of this
/// suite at once that is the slowest thing in the room, and a deadline sized
/// for the steady state fails on the setup rather than on the program. The
/// allowance for a test is 5 s, so this is the most one wait can ask for
/// without spending the whole test on itself.
const PICKER_TIMEOUT: Duration = Duration::from_millis(4500);

/// Wait for the picker to be drawn, and hand back what it shows.
fn wait_for_picker(pty: &mut Pty) -> String {
    pty.wait_until("the Inline Picker", PICKER_TIMEOUT, |screen| {
        screen.contents().contains("Select a Connection")
    })
    .expect("the binding opened the Inline Picker");
    pty.wait_for_frame().expect("the picker is drawn");
    pty.grid()
}

// — zsh —

/// The binding opens the picker, seeded from what is already typed.
#[test]
fn zsh_binding_opens_the_picker_on_the_buffer() {
    let (_fixture, mut pty) = open_picker(&ZSH, "web");
    let grid = wait_for_picker(&mut pty);
    assert!(
        grid.contains("web-01"),
        "the buffer is the picker's query\n{grid}"
    );
    assert!(
        !grid.contains("db-01"),
        "and the query really filtered: db-01 is not on offer\n{grid}"
    );
}

/// Selecting puts the alias on the prompt line, where the next Enter runs it.
///
/// The buffer holds the alias and nothing else — the same contract `$(sshm pick)`
/// has, and the reason the frame has to draw itself somewhere other than stdout.
/// The old buffer is gone, not appended to: the widget replaces the line it was
/// called from.
#[test]
fn zsh_selection_replaces_the_buffer() {
    let (_fixture, mut pty) = open_picker(&ZSH, "web");
    wait_for_picker(&mut pty);

    pty.key(b"\r");
    wait_for_buffer(&mut pty, "web-01");
}

/// Escape leaves the line exactly as it was found.
///
/// The keystroke is bound to a line the user is in the middle of typing. A
/// cancel that ate the half-typed command would make the binding something to be
/// afraid of, which is the whole reason the widget saves the buffer first.
#[test]
fn zsh_escape_leaves_the_buffer_alone() {
    let (_fixture, mut pty) = open_picker(&ZSH, "git status");
    wait_for_picker(&mut pty);

    pty.key(b"\x1b");
    wait_for_buffer(&mut pty, "git status");
}

// — bash —

/// The binding opens the picker, seeded from what is already typed.
#[test]
fn bash_binding_opens_the_picker_on_the_buffer() {
    let (_fixture, mut pty) = open_picker(&BASH, "web");
    let grid = wait_for_picker(&mut pty);
    assert!(
        grid.contains("web-01"),
        "the buffer is the picker's query\n{grid}"
    );
    assert!(
        !grid.contains("db-01"),
        "and the query really filtered: db-01 is not on offer\n{grid}"
    );
}

/// Selecting puts the alias on the prompt line.
#[test]
fn bash_selection_replaces_the_buffer() {
    let (_fixture, mut pty) = open_picker(&BASH, "web");
    wait_for_picker(&mut pty);

    pty.key(b"\r");
    wait_for_buffer(&mut pty, "web-01");
}

/// Escape leaves the line exactly as it was found.
#[test]
fn bash_escape_leaves_the_buffer_alone() {
    let (_fixture, mut pty) = open_picker(&BASH, "git status");
    wait_for_picker(&mut pty);

    pty.key(b"\x1b");
    wait_for_buffer(&mut pty, "git status");
}
