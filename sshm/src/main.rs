// The bin is a thin CLI over the library: it uses the same modules the tests
// drive, rather than compiling a second private copy of them.
use sshm::{
    config, connections, emit,
    frame::FrameMode,
    inline,
    manage::{ManageState, Phase},
    ssh, update,
};

use std::io::{self, Write};

use clap::{CommandFactory, Parser, Subcommand};
use emit::{Action, Emit};
use update::{ApplyResult, UpdateResult};

#[derive(Parser)]
#[command(name = "sshm")]
#[command(about = "A modern TUI for managing SSH connections")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short = 'c', long = "check-update", global = true)]
    check_update: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new SSH connection
    Add {
        /// Connection alias (friendly name)
        #[arg(short = 'a', long)]
        alias: Option<String>,

        /// Server hostname (optionally with user@host format)
        #[arg(required = true)]
        host: Option<String>,

        /// SSH username
        #[arg(short = 'u', long)]
        user: Option<String>,

        /// SSH port (default: 22)
        #[arg(short = 'p', long, default_value = "22")]
        port: u16,

        /// Path to private key
        #[arg(short = 'k', long)]
        key: Option<String>,

        /// Folder/group for organization
        #[arg(short = 'f', long)]
        folder: Option<String>,
    },

    /// Generate shell completion scripts
    Completions {
        /// Shell type to generate completions for
        #[arg(value_parser = clap::value_parser!(clap_complete::Shell))]
        shell: clap_complete::Shell,
    },

    /// Generate shell initialization scripts
    Init {
        /// Shell type to generate initialization script for
        #[arg(value_parser = clap::value_parser!(ShellType))]
        shell: ShellType,

        /// Suppress bind lines in the generated script (env: SSHM_NO_BIND)
        #[arg(long)]
        no_bind: bool,
    },

    /// Check for updates
    CheckUpdate,

    /// Replace the installed binary with the latest release
    Update,

    /// Open an inline picker to select a Connection (insert, don't execute)
    Pick {
        /// Seed the picker with an initial fuzzy query (for shell widget integration)
        #[arg(long)]
        query: Option<String>,
    },

    /// Open the shared inline frame in manage mode (Enter edits the Connection)
    Manage,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum ShellType {
    Zsh,
    Bash,
}

/// Runs the CLI. An error is one line on stderr and exit 1: the message is
/// printed by its `Display`, not its `Debug`, because these messages are for
/// a user at a terminal — `Error: Custom { kind: Uncategorized, error: "…" }`
/// is how an `io::Result` from `main` reports them, and it hides the one fact
/// the user needs (#46).
fn main() -> std::process::ExitCode {
    match run_main(
        run_frame_command,
        update::force_check_for_update,
        update::cached_update_version,
        update::apply_update,
    ) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run_main(
    run_frame_fn: fn(Emit, &mut dyn connections::Store, String, Option<String>) -> io::Result<()>,
    check_update_fn: fn() -> UpdateResult,
    read_note_fn: fn() -> Option<String>,
    apply_update_fn: fn() -> ApplyResult,
) -> io::Result<()> {
    let cli = Cli::parse();
    dispatch(
        cli,
        run_frame_fn,
        check_update_fn,
        read_note_fn,
        apply_update_fn,
    )
}

/// Dispatch on an already-parsed CLI. Separated from `run_main` so tests can
/// drive every command — and assert which point on the emit axis each one
/// lands on, and that the update checker is never reached on the frame
/// paths — without relying on `std::env::args`.
///
/// Bare `sshm`, `sshm pick` and `sshm manage` all open the same inline
/// frame; they differ only in the [`Emit`] they hand it, which is what
/// Enter then means (#35). The frame is handed the whole `Config` as a
/// [`Store`]: a manage delete must persist through the one implementation
/// `connections.rs` owns, so the seam carries the live set, not a snapshot.
///
/// `read_note_fn` is the cache-only update-note reader (#39). Each frame
/// path calls it exactly once, before the frame opens, and hands the result
/// down as data — so the paint path reads a local file at most and never
/// touches the network. The network-touching `check_update_fn` is reached
/// only on the `--check-update` path, never on a frame path; the two readers
/// are separate injected functions precisely so a test can prove that
/// separation (see `check_update_must_not_run`).
///
/// `apply_update_fn` is **Apply Update** (#46) — the only function reachable
/// from here that rewrites a file on disk, which is why it is injected like
/// the other three: no dispatch test can replace a real binary.
fn dispatch(
    cli: Cli,
    run_frame_fn: fn(Emit, &mut dyn connections::Store, String, Option<String>) -> io::Result<()>,
    check_update_fn: fn() -> UpdateResult,
    read_note_fn: fn() -> Option<String>,
    apply_update_fn: fn() -> ApplyResult,
) -> io::Result<()> {
    // Handle completions command
    if let Some(Commands::Completions { shell }) = cli.command {
        generate_completions(shell);
        return Ok(());
    }

    // Handle init command
    if let Some(Commands::Init { shell, no_bind }) = cli.command {
        // The --no-bind flag overrides the env var when set.
        if no_bind {
            std::env::set_var("SSHM_NO_BIND", "1");
        }
        match shell {
            ShellType::Zsh => {
                print_init_zsh_script();
                return Ok(());
            }
            ShellType::Bash => {
                print_init_bash_script();
                return Ok(());
            }
        }
    }

    // `sshm update` — Apply Update (#46). Handled with the other non-frame
    // paths, and *before* the single cached note read, so it can never
    // pollute a captured stdout and can never fall through into a frame.
    // Every branch returns.
    if let Some(Commands::Update) = cli.command {
        return match apply_update_fn() {
            ApplyResult::Applied { from, to } => {
                println!("sshm {from} → {to}");
                Ok(())
            }
            ApplyResult::UpToDate { version } => {
                println!("sshm {version} is already the latest release.");
                Ok(())
            }
            ApplyResult::Unwritable { path } => Err(io::Error::other(unwritable_text(&path))),
            ApplyResult::DevBuild { manifest_dir } => {
                Err(io::Error::other(dev_build_text(&manifest_dir)))
            }
            ApplyResult::Error(reason) => Err(io::Error::other(retryable_text(&reason))),
        };
    }

    // `sshm pick` — the insert emit. Runs before the update checker so a
    // captured stdout is never polluted by it, and returns straight from
    // the frame. The note is read from cache here, once, before the frame
    // opens (#39).
    //
    // The note is read once after the non-frame paths (completions, init)
    // have returned, so all three frame paths (pick, manage, bare) share
    // a single cache read. The `--check-update` path below calls
    // `check_update_fn` (the network checker), not this cached reader.
    let note = read_note_fn();

    if let Some(Commands::Pick { query }) = cli.command {
        let mut config = config::Config::load();
        return run_frame_fn(Emit::Insert, &mut config, query.unwrap_or_default(), note);
    }

    // `sshm manage` — the edit emit, in the manage frame.
    if let Some(Commands::Manage) = cli.command {
        let mut config = config::Config::load();
        return run_frame_fn(Emit::Edit, &mut config, String::new(), note);
    }

    // `sshm check-update` / `-c` — purely a question (#46). It reports
    // whether a newer release exists and names the command that installs it;
    // it downloads nothing and writes no binary anywhere. Every branch
    // returns: the error branch used to print and fall through into the `add`
    // handling and then a frame, which is acceptable for nobody and absurd
    // after a command that only asked a question.
    if cli.check_update || matches!(cli.command, Some(Commands::CheckUpdate)) {
        return write_check_answer(&mut io::stdout(), &check_update_fn()).map_err(io::Error::other);
    }

    // Handle add command
    if let Some(Commands::Add {
        alias,
        host,
        user,
        port,
        key,
        folder,
    }) = cli.command
    {
        run_add_command(alias, host, user, port, key, folder).map_err(io::Error::other)?;
        return Ok(());
    }

    // Bare `sshm` — the execute emit. There is no fullscreen to fall
    // through to: the frame is the whole surface now. The note was read
    // once above, before the frame opens (#39).
    let mut config = config::Config::load();
    run_frame_fn(Emit::Execute, &mut config, String::new(), note)
}

// The three ways Apply Update can fail to replace the binary, and the text
// each one prints.
//
// Named for the failure rather than switched on the result a second time, and
// returning a String rather than calling `eprintln!`, so the contract is
// assertable: the `install.sh` escape hatch appears on exactly one of them —
// the failure Apply Update can never work around. A network blip, a rate limit
// and a missing asset all want a retry, and printing the reinstall incantation
// beside them would train users to reach for the sledgehammer every time
// GitHub hiccups (#46).

/// The install location cannot be written: name it, and name the way out.
fn unwritable_text(path: &std::path::Path) -> String {
    format!(
        "cannot update: {} is not writable by this user.\n\
         Nothing was downloaded and nothing was changed. To update, install the latest release with:\n\
         \x20 curl -sL https://raw.githubusercontent.com/titonio/ssh-manager/master/install.sh | bash",
        path.display()
    )
}

/// A Dev Build is refused on principle: the variable is the whole explanation,
/// and `install.sh` is not the answer to "you are running from a checkout".
fn dev_build_text(manifest_dir: &str) -> String {
    format!(
        "refusing to update a Dev Build: CARGO_MANIFEST_DIR is set to {} — this binary \
         is running from a source checkout rather than an installed location.\n\
         sshm never replaces a binary the developer owns.",
        manifest_dir
    )
}

/// Write the answer Check for Updates gives, and return the reason when there
/// is no answer.
///
/// The sink is a parameter so a test can read the answer without a child
/// process — which is what makes "the answer names `sshm update`" an assertion
/// rather than a hope. The Update Note above the frame says `run sshm update`;
/// the terminal that answers the question has to agree with it (#46).
fn write_check_answer(out: &mut dyn Write, result: &UpdateResult) -> Result<(), String> {
    match result {
        UpdateResult::UpdateAvailable { version } => writeln!(out, "Update available: v{version}")
            .and_then(|()| writeln!(out, "Install it with: sshm update"))
            .map_err(|e| e.to_string()),
        UpdateResult::NoUpdate => writeln!(out, "No update available.").map_err(|e| e.to_string()),
        // Framed here, printed by `main`: the reason travels, and the wording
        // is not doubled on the way.
        UpdateResult::Error(reason) => Err(format!("Error checking for updates: {reason}")),
    }
}

/// Everything else — network, rate limit, missing asset: keep the reason, ask
/// for a retry, offer nothing else.
fn retryable_text(reason: &str) -> String {
    format!("update failed: {reason}")
}

/// The first-run import offer's predicate (#38): what an import from
/// `ssh_config_path` would add, if the store is empty and the answer is
/// not nothing.
///
/// `Some(count)` is the whole decision the frame runner makes before
/// manage opens. Two conditions, both honest:
///
/// * the Connection set is empty — a user who already has Connections
///   was never a first run, and offering to bulk-write over a set they
///   built by hand is not this feature;
/// * the file has at least one importable stanza — an absent or
///   all-wildcard `~/.ssh/config` has nothing to offer, and offering
///   zero is offering nothing.
///
/// The path is a parameter rather than a resolved home dir so the
/// predicate is testable against a fixture with no `HOME` faking: the
/// caller owns the one honest answer to "where is the user's ssh
/// config", and this owns the answer to "is there something to offer
/// at a given path".
fn first_run_import_offer(store: &dyn connections::Store, ssh_config_path: &str) -> Option<usize> {
    if !store.all().is_empty() {
        return None;
    }
    let count = config::count_importable(store.all(), ssh_config_path);
    (count > 0).then_some(count)
}

/// The real frame command: open the shared inline frame, resolve the
/// outcome along the emit axis, and perform the resulting action.
///
/// The frame never writes to a captured stdout. When stdout is a pipe —
/// `out=$(sshm pick)` — the frame is drawn to `/dev/tty` instead, so the
/// only bytes the pipe ever carries are the ones this function deliberately
/// emits. Keys already come from the terminal: crossterm reads stdin when
/// it is a tty and opens `/dev/tty` itself when it is not.
fn run_frame_command(
    emit: Emit,
    store: &mut dyn connections::Store,
    query: String,
    note: Option<String>,
) -> io::Result<()> {
    let mut frame_out = open_frame_stream()?;

    // The first-run import offer (#38) belongs to manage only: the pick
    // frame asks no questions, and an import is a write. When manage
    // opens on an empty set with importable stanzas in ~/.ssh/config,
    // the frame opens on the offer instead of the list — seeded as a
    // `ManageState`, because the offer is a step the frame answers, not
    // a flag the driver re-derives. Every other path is untouched.
    let offer = if emit.frame_mode() == FrameMode::Manage {
        config::ssh_config_path()
            .filter(|path| path.exists())
            .and_then(|path| {
                let path = path.to_string_lossy().into_owned();
                first_run_import_offer(store, &path).map(|count| (count, path))
            })
    } else {
        None
    };

    let outcome = match offer {
        Some((count, path)) => {
            let initial = ManageState {
                phase: Phase::ConfirmImport { count, path },
                ..Default::default()
            };
            inline::run_inline_with_state(&mut frame_out, store, initial, emit.frame_mode(), note)?
        }
        None => inline::run_inline(&mut frame_out, store, query, emit.frame_mode(), note)?,
    };

    match emit.resolve(outcome) {
        Action::Execute(args) => {
            let code = ssh::execute_ssh(&args);
            std::process::exit(code);
        }
        Action::Insert(alias) => {
            // The emitted selection: the alias alone, on the real stdout.
            println!("{alias}");
            Ok(())
        }
        Action::Edit(_) => {
            // Nothing to write. The frame's own collapse already left the
            // single settle line story 13 asks for, and this is the whole
            // cut-over contract for `sshm manage`: Enter routes the
            // selection to the edit path instead of to `ssh`. The edit
            // itself is #36/#37 — until they land no file is written here,
            // so the runner adds no verb claiming that something changed
            // (the `◆ editing` it used to print sat on top of the frame's
            // own `◆ picked`: one keystroke, two settle lines).
            Ok(())
        }
        Action::Cancelled => {
            std::process::exit(130);
        }
    }
}

/// The stream the frame draws to: stdout when it is a real terminal,
/// `/dev/tty` when stdout is captured and reserved for the emitted
/// selection.
///
/// The decision itself is [`emit::frame_stream`] — the emit axis's own
/// seam — so the routing lives with the rest of the emit policy and this
/// function only opens the stream that decision names. `run_inline` then
/// draws to whatever it is handed (rule 4: one stream, both directions),
/// which is what keeps the captured pipe free of control sequences.
fn open_frame_stream() -> io::Result<Box<dyn Write>> {
    use std::io::IsTerminal;
    match emit::frame_stream(std::io::stdout().is_terminal()) {
        emit::FrameStream::Stdout => Ok(Box::new(io::stdout())),
        emit::FrameStream::Tty => Ok(Box::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/tty")?,
        )),
    }
}

fn run_add_command(
    alias: Option<String>,
    host: Option<String>,
    user: Option<String>,
    port: u16,
    key_path: Option<String>,
    folder: Option<String>,
) -> Result<(), String> {
    let alias = alias.ok_or("Error: --alias is required")?;
    let host = host.ok_or("Error: user@host is required")?;

    let conn = config::Connection {
        id: uuid::Uuid::new_v4().to_string(),
        alias,
        host,
        user: user.unwrap_or_default(),
        port,
        key_path,
        folder,
    };

    let mut config = config::Config::load();
    config.add_connection(conn);
    config
        .save()
        .map_err(|e| format!("Error saving config: {}", e))?;

    println!("Connection added successfully!");

    Ok(())
}

fn generate_completions(shell: clap_complete::Shell) {
    let mut app = Cli::command();
    let bin_name = app.get_name().to_string();

    println!("Generating completion script for {:?}...", shell);

    clap_complete::generate(shell, &mut app, bin_name, &mut std::io::stdout());
}

/// Generate the zsh initialization script for the inline picker widget and
/// **<TAB> completion trigger.
///
/// Returns the script content as a string for testing and printing.
fn generate_init_zsh_script() -> String {
    // Default bind key (Ctrl+Alt+S)
    // Default bind key for Ctrl+Alt+S in zsh notation: `\e` is ESC (the
    // Alt/Meta modifier prefix) and `^S` is Ctrl+S, yielding Ctrl+Alt+S. This
    // matches the legacy xterm encoding ESC + Ctrl-S that the terminals send,
    // and is far more portable than a guessed CSI sequence.
    let default_bind_key = "\\e^S";

    // Check for SSHM_NO_BIND environment variable
    let no_bind = std::env::var("SSHM_NO_BIND").is_ok();

    // Check for custom bind key via SSHM_BIND_KEY environment variable
    let bind_key = std::env::var("SSHM_BIND_KEY").unwrap_or_else(|_| default_bind_key.to_string());

    if no_bind {
        r#"# sshm init zsh - Inline Picker Widget + **<TAB> completion trigger (no bind)
# Sourced via: eval "$(sshm init zsh)"
# Note: Bind lines suppressed by SSHM_NO_BIND=1

# Shell function wrapper: makes "sshm pick" insert onto the command line
# when run interactively (stdout is a terminal).  For pipe/redirect usage the
# raw binary output is preserved.
sshm() {
    if [[ "$1" == "pick" && -t 1 ]]; then
        local result
        result=$(command sshm pick "${@:2}")
        local ret=$?
        if [[ $ret -eq 0 && -n "$result" ]]; then
            print -z "$result"
        fi
        return $ret
    fi
    command sshm "$@"
}

# ZLE widget function for sshm inline picker (Ctrl+Alt+S)
_sshm_inline_picker() {
    local saved_buffer="$BUFFER"
    
    local result
    result=$(sshm pick --query "$BUFFER")
    local exit_code=$?
    
    if [[ $exit_code -ne 0 ]]; then
        BUFFER="$saved_buffer"
        zle reset-prompt
        return
    fi
    
    if [[ -n "$result" ]]; then
        BUFFER="$result"
        CURSOR=${#BUFFER}
    fi
    
    zle reset-prompt
}

# **<TAB> completion trigger: when LBUFFER ends with **, pressing Tab opens
# the inline picker instead of running normal zsh completion
_sshm_completion_picker() {
    if [[ $LBUFFER == *'**' ]]; then
        local saved_buffer="$BUFFER"
        local stripped="${LBUFFER%'**'}"
        LBUFFER="$stripped"
        
        local result
        result=$(sshm pick --query "$stripped")
        local exit_code=$?
        
        if [[ $exit_code -ne 0 ]]; then
            BUFFER="$saved_buffer"
            zle reset-prompt
            return
        fi
        
        if [[ -n "$result" ]]; then
            BUFFER="$result"
            CURSOR=${#BUFFER}
        fi
        
        zle reset-prompt
    else
        zle expand-or-complete
    fi
}

# Register the ZLE widgets
zle -N _sshm_inline_picker
zle -N _sshm_completion_picker
"#
        .to_string()
    } else {
        format!(
            r#"# sshm init zsh - Inline Picker Widget + **<TAB> completion trigger
# Sourced via: eval "$(sshm init zsh)"
# Bind key: {} (override with SSHM_BIND_KEY)

# Shell function wrapper: makes "sshm pick" insert onto the command line
# when run interactively (stdout is a terminal).  For pipe/redirect usage the
# raw binary output is preserved.
sshm() {{
    if [[ "$1" == "pick" && -t 1 ]]; then
        local result
        result=$(command sshm pick "${{@:2}}")
        local ret=$?
        if [[ $ret -eq 0 && -n "$result" ]]; then
            print -z "$result"
        fi
        return $ret
    fi
    command sshm "$@"
}}

# ZLE widget function for sshm inline picker (Ctrl+Alt+S)
_sshm_inline_picker() {{
    local saved_buffer="$BUFFER"
    
    local result
    result=$(sshm pick --query "$BUFFER")
    local exit_code=$?
    
    if [[ $exit_code -ne 0 ]]; then
        BUFFER="$saved_buffer"
        zle reset-prompt
        return
    fi
    
    if [[ -n "$result" ]]; then
        BUFFER="$result"
        CURSOR=${{#BUFFER}}
    fi
    
    zle reset-prompt
}}

# **<TAB> completion trigger: when LBUFFER ends with **, pressing Tab opens
# the inline picker instead of running normal zsh completion
_sshm_completion_picker() {{
    if [[ $LBUFFER == *'**' ]]; then
        local saved_buffer="$BUFFER"
        local stripped="${{LBUFFER%'**'}}"
        LBUFFER="$stripped"
        
        local result
        result=$(sshm pick --query "$stripped")
        local exit_code=$?
        
        if [[ $exit_code -ne 0 ]]; then
            BUFFER="$saved_buffer"
            zle reset-prompt
            return
        fi
        
        if [[ -n "$result" ]]; then
            BUFFER="$result"
            CURSOR=${{#BUFFER}}
        fi
        
        zle reset-prompt
    else
        zle expand-or-complete
    fi
}}

# Register the ZLE widgets
zle -N _sshm_inline_picker
zle -N _sshm_completion_picker

# Bind the widget to the trigger key (Ctrl+Alt+S)
bindkey '{}' _sshm_inline_picker

# Bind **<TAB> completion trigger (Tab key)
bindkey '^I' _sshm_completion_picker
"#,
            bind_key, bind_key
        )
    }
}

/// Print the zsh initialization script for the inline picker widget.
fn print_init_zsh_script() {
    print!("{}", generate_init_zsh_script());
}

/// Generate the bash initialization script for the inline picker widget.
///
/// TAB is deliberately never rebound here: readline cannot chain from a
/// `bind -x` function back to normal completion, so binding TAB kills it
/// for the whole session (#23). The `**<TAB>` trigger is zsh-only; bash
/// users enter the picker via the bound key (Ctrl+Alt+S by default).
///
/// Returns the script content as a string for testing and printing.
fn generate_init_bash_script() -> String {
    // Default bind key in bash bind -x notation for Ctrl+Alt+S.
    // bash uses `\e` for the Escape key (Alt/Meta prefix) and `\C-s` for Ctrl+S.
    // Together `"\e\C-s"` represents Ctrl+Alt+S, matching the legacy xterm
    // encoding ESC + Ctrl-S that terminals send.
    let default_bind_key = "\\e\\C-s";

    // Check for SSHM_NO_BIND environment variable
    let no_bind = std::env::var("SSHM_NO_BIND").is_ok();

    // Check for custom bind key via SSHM_BIND_KEY environment variable
    let bind_key = std::env::var("SSHM_BIND_KEY").unwrap_or_else(|_| default_bind_key.to_string());

    if no_bind {
        r###"# sshm init bash - Inline Picker Widget (no bind)
# Sourced via: eval "$(sshm init bash)"
# Note: Bind lines suppressed by SSHM_NO_BIND=1

_sshm_inline_picker() {
    local result exit_code
    result=$(sshm pick --query "$READLINE_LINE")
    exit_code=$?
    if [[ $exit_code -ne 0 ]]; then
        return
    fi
    if [[ -n "$result" ]]; then
        READLINE_LINE="$result"
        READLINE_POINT=${#READLINE_LINE}
    fi
}
"###
        .to_string()
    } else {
        format!(
            r###"# sshm init bash - Inline Picker Widget
# Sourced via: eval "$(sshm init bash)"
# Bind key: {bind_key} (override with SSHM_BIND_KEY)

_sshm_inline_picker() {{
    local result exit_code
    result=$(sshm pick --query "$READLINE_LINE")
    exit_code=$?
    if [[ $exit_code -ne 0 ]]; then
        return
    fi
    if [[ -n "$result" ]]; then
        READLINE_LINE="$result"
        READLINE_POINT=${{#READLINE_LINE}}
    fi
}}

# Bind the widget to the trigger key (Ctrl+Alt+S)
bind -x '"{bind_key}":_sshm_inline_picker'
"###
        )
    }
}

/// Print the bash initialization script for the inline picker widget.
fn print_init_bash_script() {
    print!("{}", generate_init_bash_script());
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serial_test::serial;

    // The frame runner is a fn pointer, so it records into a thread-local
    // rather than a closure: the assertion is then about what each command
    // *asked the frame to be*, which is the routing, not a side effect.
    #[derive(Debug, Clone, PartialEq)]
    struct FrameCall {
        emit: Emit,
        query: String,
        note: Option<String>,
    }

    thread_local! {
        static FRAME_CALLS: std::cell::RefCell<Vec<FrameCall>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    fn record_frame(
        emit: Emit,
        _store: &mut dyn connections::Store,
        query: String,
        note: Option<String>,
    ) -> io::Result<()> {
        FRAME_CALLS.with(|c| c.borrow_mut().push(FrameCall { emit, query, note }));
        Ok(())
    }

    fn take_calls() -> Vec<FrameCall> {
        FRAME_CALLS.with(|c| std::mem::take(&mut *c.borrow_mut()))
    }

    fn check_update_must_not_run() -> UpdateResult {
        panic!("the frame paths must never reach the update checker");
    }

    /// The frame runner for the paths that must never paint one: reaching it
    /// *is* the failure (#46).
    fn frame_must_not_run(
        _emit: Emit,
        _store: &mut dyn connections::Store,
        _query: String,
        _note: Option<String>,
    ) -> io::Result<()> {
        panic!("neither update command may open a frame");
    }

    /// The applier for every path that must not act. Reaching it is the
    /// failure: it is the one function reachable from dispatch that rewrites
    /// a file on disk, so no frame path — and no check — is allowed anywhere
    /// near it (#46).
    fn apply_update_must_not_run() -> ApplyResult {
        panic!("only `sshm update` may reach the applier");
    }

    thread_local! {
        static APPLY_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// The applier most dispatch tests use: it records that it was reached
    /// and reports success. No dispatch test ever reaches the real applier,
    /// which is the only code that can replace a binary (#46).
    fn record_apply() -> ApplyResult {
        APPLY_CALLS.with(|c| c.set(c.get() + 1));
        ApplyResult::Applied {
            from: "0.1.12".to_string(),
            to: "0.1.13".to_string(),
        }
    }

    fn take_applies() -> usize {
        APPLY_CALLS.with(|c| c.replace(0))
    }

    /// The cache-only note reader most frame tests use: no cached version,
    /// so no note. Distinct from `check_update_must_not_run` — this one is
    /// *expected* to run on the frame path (it reads a file, not the
    /// network), it just has nothing to report here.
    fn no_cached_note() -> Option<String> {
        None
    }

    /// A cache reader that reports a version, for the test that proves the
    /// frame path carries the note through to the frame.
    fn cached_note_present() -> Option<String> {
        Some("0.2.0".to_string())
    }

    // ── the three commands, one frame, three emits (#35) ────────────────

    #[test]
    fn bare_sshm_opens_the_frame_on_the_execute_emit() {
        let cli = Cli {
            command: None,
            check_update: false,
        };
        dispatch(
            cli,
            record_frame,
            check_update_must_not_run,
            no_cached_note,
            apply_update_must_not_run,
        )
        .unwrap();
        assert_eq!(
            take_calls(),
            vec![FrameCall {
                emit: Emit::Execute,
                query: String::new(),
                note: None,
            }]
        );
    }

    #[test]
    fn pick_opens_the_frame_on_the_insert_emit_with_its_query() {
        let cli = Cli {
            command: Some(Commands::Pick {
                query: Some("web".to_string()),
            }),
            check_update: false,
        };
        dispatch(
            cli,
            record_frame,
            check_update_must_not_run,
            no_cached_note,
            apply_update_must_not_run,
        )
        .unwrap();
        assert_eq!(
            take_calls(),
            vec![FrameCall {
                emit: Emit::Insert,
                query: "web".to_string(),
                note: None,
            }]
        );
    }

    #[test]
    fn manage_opens_the_frame_on_the_edit_emit() {
        let cli = Cli {
            command: Some(Commands::Manage),
            check_update: false,
        };
        dispatch(
            cli,
            record_frame,
            check_update_must_not_run,
            no_cached_note,
            apply_update_must_not_run,
        )
        .unwrap();
        assert_eq!(
            take_calls(),
            vec![FrameCall {
                emit: Emit::Edit,
                query: String::new(),
                note: None,
            }]
        );
    }

    #[test]
    fn pick_without_a_query_seeds_the_frame_with_an_empty_one() {
        let cli = Cli {
            command: Some(Commands::Pick { query: None }),
            check_update: false,
        };
        dispatch(
            cli,
            record_frame,
            check_update_must_not_run,
            no_cached_note,
            apply_update_must_not_run,
        )
        .unwrap();
        assert_eq!(
            take_calls(),
            vec![FrameCall {
                emit: Emit::Insert,
                query: String::new(),
                note: None,
            }]
        );
    }

    // ── the cached update note reaches the frame, never the network (#39) ─

    /// The frame path reads the note from the injected cache reader and
    /// carries it to the frame. The network-touching checker is
    /// `check_update_must_not_run`, which panics — so the test passing at
    /// all *is* the proof that painting the frame never touches the
    /// network. This is the `check_update_must_not_run` idiom extended to
    /// the note: a cache-only reader is used, the network checker is not.
    #[test]
    fn the_frame_path_carries_the_cached_note_and_never_touches_the_network() {
        let cli = Cli {
            command: None,
            check_update: false,
        };
        dispatch(
            cli,
            record_frame,
            check_update_must_not_run,
            cached_note_present,
            apply_update_must_not_run,
        )
        .unwrap();
        assert_eq!(
            take_calls(),
            vec![FrameCall {
                emit: Emit::Execute,
                query: String::new(),
                note: Some("0.2.0".to_string()),
            }],
            "the cached note must reach the frame on the bare-sshm path"
        );
    }

    /// The same on the pick path: the note rides the insert emit too.
    #[test]
    fn the_pick_frame_path_carries_the_cached_note() {
        let cli = Cli {
            command: Some(Commands::Pick {
                query: Some("web".to_string()),
            }),
            check_update: false,
        };
        dispatch(
            cli,
            record_frame,
            check_update_must_not_run,
            cached_note_present,
            apply_update_must_not_run,
        )
        .unwrap();
        assert_eq!(
            take_calls(),
            vec![FrameCall {
                emit: Emit::Insert,
                query: "web".to_string(),
                note: Some("0.2.0".to_string()),
            }]
        );
    }

    /// The same on the manage path.
    #[test]
    fn the_manage_frame_path_carries_the_cached_note() {
        let cli = Cli {
            command: Some(Commands::Manage),
            check_update: false,
        };
        dispatch(
            cli,
            record_frame,
            check_update_must_not_run,
            cached_note_present,
            apply_update_must_not_run,
        )
        .unwrap();
        assert_eq!(
            take_calls(),
            vec![FrameCall {
                emit: Emit::Edit,
                query: String::new(),
                note: Some("0.2.0".to_string()),
            }]
        );
    }

    #[test]
    fn completions_never_opens_a_frame() {
        let cli = Cli {
            command: Some(Commands::Completions {
                shell: clap_complete::Shell::Zsh,
            }),
            check_update: false,
        };
        dispatch(
            cli,
            record_frame,
            check_update_must_not_run,
            no_cached_note,
            apply_update_must_not_run,
        )
        .unwrap();
        assert!(take_calls().is_empty());
    }

    #[test]
    #[serial]
    fn init_never_opens_a_frame() {
        let cli = Cli {
            command: Some(Commands::Init {
                shell: ShellType::Zsh,
                no_bind: true,
            }),
            check_update: false,
        };
        dispatch(
            cli,
            record_frame,
            check_update_must_not_run,
            no_cached_note,
            apply_update_must_not_run,
        )
        .unwrap();
        assert!(take_calls().is_empty());
    }

    #[test]
    fn the_check_update_flag_reaches_the_checker_not_the_frame() {
        fn no_update() -> UpdateResult {
            UpdateResult::NoUpdate
        }
        let cli = Cli {
            command: None,
            check_update: true,
        };
        dispatch(
            cli,
            record_frame,
            no_update,
            no_cached_note,
            apply_update_must_not_run,
        )
        .unwrap();
        assert!(take_calls().is_empty(), "-c checks, it does not frame");
    }

    // ── Apply Update: one command that acts, and never paints (#46) ──────

    /// `sshm update` is the act, not the question: it reaches the injected
    /// applier exactly once. The frame function here panics, so the test
    /// passing at all is the proof that updating never opens a frame — and
    /// the checker function panics too, because applying is not checking.
    #[test]
    fn apply_update_reaches_the_applier_exactly_once_and_never_a_frame() {
        let cli = Cli {
            command: Some(Commands::Update),
            check_update: false,
        };
        dispatch(
            cli,
            frame_must_not_run,
            check_update_must_not_run,
            no_cached_note,
            record_apply,
        )
        .unwrap();
        assert_eq!(take_applies(), 1);
        assert!(take_calls().is_empty());
    }

    /// An up to date install is a success, not an error: exit 0, nothing to
    /// report but that fact, and no frame (#46).
    #[test]
    fn an_up_to_date_apply_is_a_success_that_never_frames() {
        fn up_to_date() -> ApplyResult {
            ApplyResult::UpToDate {
                version: "0.1.12".to_string(),
            }
        }
        let cli = Cli {
            command: Some(Commands::Update),
            check_update: false,
        };
        let before = take_calls();
        dispatch(
            cli,
            frame_must_not_run,
            check_update_must_not_run,
            no_cached_note,
            up_to_date,
        )
        .expect("nothing to do is not a failure");
        assert!(before.is_empty() && take_calls().is_empty());
    }

    /// A failed apply is a non-zero exit and no frame — the error path that
    /// used to fall through into opening one is closed (#46).
    #[test]
    fn a_failed_apply_exits_non_zero_and_never_frames() {
        fn failed() -> ApplyResult {
            ApplyResult::Error("api request failed with status: 403".to_string())
        }
        let cli = Cli {
            command: Some(Commands::Update),
            check_update: false,
        };
        let err = dispatch(
            cli,
            frame_must_not_run,
            check_update_must_not_run,
            no_cached_note,
            failed,
        )
        .expect_err("a failed update must not report success");
        assert!(
            format!("{err}").contains("403"),
            "the underlying error is surfaced unedited, got: {err}"
        );
        assert!(take_calls().is_empty());
    }

    /// A failed check is the same: non-zero, and no frame. This is the
    /// fall-through that made a network blurb print `Error: Os { code: 6 }`
    /// from a frame nobody asked for (#46).
    #[test]
    fn a_failed_check_update_exits_non_zero_and_never_frames() {
        fn failed() -> UpdateResult {
            UpdateResult::Error("Failed to fetch releases".to_string())
        }
        for cli in [
            Cli {
                command: None,
                check_update: true,
            },
            Cli {
                command: Some(Commands::CheckUpdate),
                check_update: false,
            },
        ] {
            dispatch(
                cli,
                frame_must_not_run,
                failed,
                no_cached_note,
                apply_update_must_not_run,
            )
            .expect_err("a failed check must not report success");
            assert!(take_calls().is_empty());
        }
    }

    // ── what each failure says, and where the escape hatch appears (#46) ──

    /// Permission is the one failure Apply Update can never work around, so
    /// it is the only one that prints `install.sh` — and it must name the
    /// binary it could not write, or the user cannot act on it.
    #[test]
    fn an_unwritable_install_location_names_the_binary_and_offers_install_sh() {
        let text = unwritable_text(std::path::Path::new("/usr/local/bin/sshm"));
        assert!(
            text.contains("/usr/local/bin/sshm"),
            "the unwritable path must be named: {text}"
        );
        assert!(
            text.contains("install.sh"),
            "the escape hatch belongs here: {text}"
        );
    }

    /// A Dev Build is refused on principle, not on permissions. The variable
    /// is the whole explanation, so it must be named; `install.sh` is not the
    /// answer to "you are running from a checkout" (#46).
    #[test]
    fn a_dev_build_refusal_names_the_variable_that_marks_it() {
        let text = dev_build_text("/home/dev/ssh-manager/sshm");
        assert!(
            text.contains("CARGO_MANIFEST_DIR"),
            "a refusal must say what made it: {text}"
        );
        assert!(
            !text.contains("install.sh"),
            "a checkout is not a permissions problem: {text}"
        );
    }

    /// A network blip, a rate limit and a missing asset all want a retry,
    /// not a reinstallation. Printing `install.sh` here would train users to
    /// reach for the sledgehammer every time GitHub hiccups (#46).
    #[test]
    fn a_retryable_failure_asks_for_a_retry_and_not_for_install_sh() {
        for reason in [
            "api request failed with status: 403",
            "No asset found for target: `aarch64-apple-darwin`",
        ] {
            let text = retryable_text(reason);
            assert!(text.contains(reason), "the reason is kept: {text}");
            assert!(
                !text.contains("install.sh"),
                "no escape hatch for a retryable failure: {text}"
            );
        }
    }

    /// The answer to Check for Updates names the act that follows it. The
    /// Update Note above the frame already says `run sshm update`; a terminal
    /// that answered "Update available" and stopped there was the half-sentence
    /// this ticket is about (#46).
    #[test]
    fn the_check_answer_names_sshm_update_as_the_way_to_install() {
        let mut out = Vec::new();
        write_check_answer(
            &mut out,
            &UpdateResult::UpdateAvailable {
                version: "0.9.9".to_string(),
            },
        )
        .expect("an available update is not a failure");
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("0.9.9"), "the version is reported: {text}");
        assert!(
            text.contains("sshm update"),
            "the answer must point at the act: {text}"
        );
    }

    /// Nothing to install is the one answer that must not offer to install.
    #[test]
    fn the_no_update_answer_offers_nothing_to_install() {
        let mut out = Vec::new();
        write_check_answer(&mut out, &UpdateResult::NoUpdate).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            !text.contains("sshm update"),
            "up to date, yet told to update: {text}"
        );
    }

    /// A failed check yields no answer and carries its reason instead — the
    /// caller turns that into a non-zero exit rather than a frame (#46).
    #[test]
    fn a_failed_check_produces_no_answer_and_carries_its_reason() {
        let mut out = Vec::new();
        let err = write_check_answer(&mut out, &UpdateResult::Error("rate limited".to_string()))
            .expect_err("a failed check is not an answer");
        assert!(out.is_empty(), "a failure prints nothing on stdout");
        assert!(err.contains("rate limited"), "{err}");
    }

    // ── first-run import offer predicate (#38) ──────────────────────────
    //
    // The predicate takes the ssh config path explicitly, so these
    // fixtures need no HOME faking: the caller resolves the real
    // ~/.ssh/config, and the tests hand it a file they wrote themselves.

    fn ssh_config_fixture(name: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config");
        std::fs::write(&path, content).unwrap();
        path
    }

    fn remove_fixture(path: &std::path::Path) {
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn first_run_offer_counts_importable_stanzas_for_an_empty_store() {
        let path = ssh_config_fixture(
            "ssh-manager-first-run-offer-test",
            "Host web-01\n    HostName web.example.com\n    User www\n\nHost db-01\n    HostName db.example.com\n    User dba\n",
        );
        let store = config::Config::new();
        assert_eq!(
            first_run_import_offer(&store, path.to_str().unwrap()),
            Some(2),
            "an empty store and two importable stanzas is exactly the offer"
        );
        remove_fixture(&path);
    }

    #[test]
    fn first_run_offer_is_silent_when_the_store_already_has_a_connection() {
        let path = ssh_config_fixture(
            "ssh-manager-first-run-nonempty-test",
            "Host web-01\n    HostName web.example.com\n    User www\n",
        );
        let mut store = config::Config::new();
        store.add_connection(config::Connection {
            id: "already-here".to_string(),
            alias: "mine".to_string(),
            host: "10.0.0.1".to_string(),
            user: "me".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        });
        assert_eq!(
            first_run_import_offer(&store, path.to_str().unwrap()),
            None,
            "a user who already has Connections is not a first run, however \
             tempting the file"
        );
        remove_fixture(&path);
    }

    #[test]
    fn first_run_offer_is_silent_when_the_file_does_not_exist() {
        let missing = std::env::temp_dir()
            .join("ssh-manager-first-run-missing-test")
            .join("config");
        let store = config::Config::new();
        assert_eq!(
            first_run_import_offer(&store, missing.to_str().unwrap()),
            None,
            "no file means nothing to offer"
        );
    }

    #[test]
    fn first_run_offer_is_silent_when_every_stanza_is_a_wildcard() {
        let path = ssh_config_fixture(
            "ssh-manager-first-run-wildcard-test",
            "Host *\n    User default\n\nHost *.internal\n    User internal\n",
        );
        let store = config::Config::new();
        assert_eq!(
            first_run_import_offer(&store, path.to_str().unwrap()),
            None,
            "a count of zero is not an offer: nothing there could become a \
             Connection"
        );
        remove_fixture(&path);
    }

    // ── sshm init zsh snapshot tests (AC8, AC9) ──────────────────────────────────────────────

    #[test]
    #[serial]
    fn test_snapshot_init_zsh_default() {
        // Clear any env overrides
        std::env::remove_var("SSHM_NO_BIND");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_zsh_script();
        insta::assert_snapshot!(script, @r###"
        # sshm init zsh - Inline Picker Widget + **<TAB> completion trigger
        # Sourced via: eval "$(sshm init zsh)"
        # Bind key: \e^S (override with SSHM_BIND_KEY)

        # Shell function wrapper: makes "sshm pick" insert onto the command line
        # when run interactively (stdout is a terminal).  For pipe/redirect usage the
        # raw binary output is preserved.
        sshm() {
            if [[ "$1" == "pick" && -t 1 ]]; then
                local result
                result=$(command sshm pick "${@:2}")
                local ret=$?
                if [[ $ret -eq 0 && -n "$result" ]]; then
                    print -z "$result"
                fi
                return $ret
            fi
            command sshm "$@"
        }

        # ZLE widget function for sshm inline picker (Ctrl+Alt+S)
        _sshm_inline_picker() {
            local saved_buffer="$BUFFER"
            
            local result
            result=$(sshm pick --query "$BUFFER")
            local exit_code=$?
            
            if [[ $exit_code -ne 0 ]]; then
                BUFFER="$saved_buffer"
                zle reset-prompt
                return
            fi
            
            if [[ -n "$result" ]]; then
                BUFFER="$result"
                CURSOR=${#BUFFER}
            fi
            
            zle reset-prompt
        }

        # **<TAB> completion trigger: when LBUFFER ends with **, pressing Tab opens
        # the inline picker instead of running normal zsh completion
        _sshm_completion_picker() {
            if [[ $LBUFFER == *'**' ]]; then
                local saved_buffer="$BUFFER"
                local stripped="${LBUFFER%'**'}"
                LBUFFER="$stripped"
                
                local result
                result=$(sshm pick --query "$stripped")
                local exit_code=$?
                
                if [[ $exit_code -ne 0 ]]; then
                    BUFFER="$saved_buffer"
                    zle reset-prompt
                    return
                fi
                
                if [[ -n "$result" ]]; then
                    BUFFER="$result"
                    CURSOR=${#BUFFER}
                fi
                
                zle reset-prompt
            else
                zle expand-or-complete
            fi
        }

        # Register the ZLE widgets
        zle -N _sshm_inline_picker
        zle -N _sshm_completion_picker

        # Bind the widget to the trigger key (Ctrl+Alt+S)
        bindkey '\e^S' _sshm_inline_picker

        # Bind **<TAB> completion trigger (Tab key)
        bindkey '^I' _sshm_completion_picker
        "###);
    }

    #[test]
    #[serial]
    fn test_snapshot_init_zsh_no_bind() {
        // Set SSHM_NO_BIND to suppress bind lines
        std::env::set_var("SSHM_NO_BIND", "1");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_zsh_script();
        insta::assert_snapshot!(script, @r###"
        # sshm init zsh - Inline Picker Widget + **<TAB> completion trigger (no bind)
        # Sourced via: eval "$(sshm init zsh)"
        # Note: Bind lines suppressed by SSHM_NO_BIND=1

        # Shell function wrapper: makes "sshm pick" insert onto the command line
        # when run interactively (stdout is a terminal).  For pipe/redirect usage the
        # raw binary output is preserved.
        sshm() {
            if [[ "$1" == "pick" && -t 1 ]]; then
                local result
                result=$(command sshm pick "${@:2}")
                local ret=$?
                if [[ $ret -eq 0 && -n "$result" ]]; then
                    print -z "$result"
                fi
                return $ret
            fi
            command sshm "$@"
        }

        # ZLE widget function for sshm inline picker (Ctrl+Alt+S)
        _sshm_inline_picker() {
            local saved_buffer="$BUFFER"
            
            local result
            result=$(sshm pick --query "$BUFFER")
            local exit_code=$?
            
            if [[ $exit_code -ne 0 ]]; then
                BUFFER="$saved_buffer"
                zle reset-prompt
                return
            fi
            
            if [[ -n "$result" ]]; then
                BUFFER="$result"
                CURSOR=${#BUFFER}
            fi
            
            zle reset-prompt
        }

        # **<TAB> completion trigger: when LBUFFER ends with **, pressing Tab opens
        # the inline picker instead of running normal zsh completion
        _sshm_completion_picker() {
            if [[ $LBUFFER == *'**' ]]; then
                local saved_buffer="$BUFFER"
                local stripped="${LBUFFER%'**'}"
                LBUFFER="$stripped"
                
                local result
                result=$(sshm pick --query "$stripped")
                local exit_code=$?
                
                if [[ $exit_code -ne 0 ]]; then
                    BUFFER="$saved_buffer"
                    zle reset-prompt
                    return
                fi
                
                if [[ -n "$result" ]]; then
                    BUFFER="$result"
                    CURSOR=${#BUFFER}
                fi
                
                zle reset-prompt
            else
                zle expand-or-complete
            fi
        }

        # Register the ZLE widgets
        zle -N _sshm_inline_picker
        zle -N _sshm_completion_picker
        "###);

        // Clean up
        std::env::remove_var("SSHM_NO_BIND");
    }

    #[test]
    #[serial]
    fn test_snapshot_init_zsh_custom_bind_key() {
        // Clear SSHM_NO_BIND and set custom SSHM_BIND_KEY
        std::env::remove_var("SSHM_NO_BIND");
        std::env::set_var("SSHM_BIND_KEY", "^S");

        let script = generate_init_zsh_script();
        insta::assert_snapshot!(script, @r###"
        # sshm init zsh - Inline Picker Widget + **<TAB> completion trigger
        # Sourced via: eval "$(sshm init zsh)"
        # Bind key: ^S (override with SSHM_BIND_KEY)

        # Shell function wrapper: makes "sshm pick" insert onto the command line
        # when run interactively (stdout is a terminal).  For pipe/redirect usage the
        # raw binary output is preserved.
        sshm() {
            if [[ "$1" == "pick" && -t 1 ]]; then
                local result
                result=$(command sshm pick "${@:2}")
                local ret=$?
                if [[ $ret -eq 0 && -n "$result" ]]; then
                    print -z "$result"
                fi
                return $ret
            fi
            command sshm "$@"
        }

        # ZLE widget function for sshm inline picker (Ctrl+Alt+S)
        _sshm_inline_picker() {
            local saved_buffer="$BUFFER"
            
            local result
            result=$(sshm pick --query "$BUFFER")
            local exit_code=$?
            
            if [[ $exit_code -ne 0 ]]; then
                BUFFER="$saved_buffer"
                zle reset-prompt
                return
            fi
            
            if [[ -n "$result" ]]; then
                BUFFER="$result"
                CURSOR=${#BUFFER}
            fi
            
            zle reset-prompt
        }

        # **<TAB> completion trigger: when LBUFFER ends with **, pressing Tab opens
        # the inline picker instead of running normal zsh completion
        _sshm_completion_picker() {
            if [[ $LBUFFER == *'**' ]]; then
                local saved_buffer="$BUFFER"
                local stripped="${LBUFFER%'**'}"
                LBUFFER="$stripped"
                
                local result
                result=$(sshm pick --query "$stripped")
                local exit_code=$?
                
                if [[ $exit_code -ne 0 ]]; then
                    BUFFER="$saved_buffer"
                    zle reset-prompt
                    return
                fi
                
                if [[ -n "$result" ]]; then
                    BUFFER="$result"
                    CURSOR=${#BUFFER}
                fi
                
                zle reset-prompt
            else
                zle expand-or-complete
            fi
        }

        # Register the ZLE widgets
        zle -N _sshm_inline_picker
        zle -N _sshm_completion_picker

        # Bind the widget to the trigger key (Ctrl+Alt+S)
        bindkey '^S' _sshm_inline_picker

        # Bind **<TAB> completion trigger (Tab key)
        bindkey '^I' _sshm_completion_picker
        "###);

        // Clean up
        std::env::remove_var("SSHM_BIND_KEY");
    }

    // ── sshm init zsh contract assertions (AC9) ──────────────────────────────────────────────

    #[test]
    fn test_init_zsh_contains_sshm_pick() {
        let script = generate_init_zsh_script();
        assert!(script.contains("sshm pick"));
    }

    #[test]
    fn test_init_zsh_seeds_query_buffer() {
        let script = generate_init_zsh_script();
        assert!(script.contains("--query \"$BUFFER\""));
    }

    #[test]
    #[serial]
    fn test_init_zsh_binds_ctrl_alt_s_by_default() {
        std::env::remove_var("SSHM_NO_BIND");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_zsh_script();
        assert!(script.contains("bindkey"));
        // \e^S is zsh notation for Ctrl+Alt+S (ESC + Ctrl+S, the legacy xterm
        // encoding). It is far more portable than a guessed CSI sequence.
        assert!(script.contains("\\e^S"));
    }

    #[test]
    #[serial]
    fn test_init_zsh_suppresses_bind_with_no_bind_flag() {
        std::env::set_var("SSHM_NO_BIND", "1");

        let script = generate_init_zsh_script();
        assert!(!script.contains("bindkey"));
        assert!(script.contains("no bind"));

        // Clean up
        std::env::remove_var("SSHM_NO_BIND");
    }

    #[test]
    #[serial]
    fn test_init_zsh_honors_sshm_bind_key() {
        std::env::remove_var("SSHM_NO_BIND");
        std::env::set_var("SSHM_BIND_KEY", "^X");

        let script = generate_init_zsh_script();
        assert!(script.contains("bindkey"));
        assert!(script.contains("'^X'"));

        // Clean up
        std::env::remove_var("SSHM_BIND_KEY");
    }

    #[test]
    fn test_init_zsh_replaces_buffer_with_cursor_to_end() {
        let script = generate_init_zsh_script();
        // Result replaces the buffer (not appended) so the full ssh command is on the command line.
        assert!(script.contains("BUFFER=\"$result\""));
        assert!(script.contains("CURSOR=${#BUFFER}"));
    }

    #[test]
    fn test_init_zsh_calls_reset_prompt() {
        let script = generate_init_zsh_script();
        assert!(script.contains("zle reset-prompt"));
    }

    #[test]
    fn test_init_zsh_restores_buffer_on_cancel() {
        let script = generate_init_zsh_script();
        assert!(script.contains("BUFFER=\"$saved_buffer\""));
    }

    // ── sshm init zsh **<TAB> completion trigger contract tests (AC for #16) ─────────────────

    #[test]
    fn test_init_zsh_has_completion_trigger_function() {
        let script = generate_init_zsh_script();
        assert!(script.contains("_sshm_completion_picker"));
    }

    #[test]
    fn test_init_zsh_completion_checks_lbuffer_for_doublestar() {
        let script = generate_init_zsh_script();
        // The widget checks if LBUFFER ends with **
        assert!(script.contains("LBUFFER == *'**'"));
    }

    #[test]
    fn test_init_zsh_completion_strips_doublestar() {
        let script = generate_init_zsh_script();
        // The widget strips ** from the end of LBUFFER
        assert!(script.contains("${LBUFFER%'**'}"));
    }

    #[test]
    fn test_init_zsh_completion_falls_through_to_expand_or_complete() {
        let script = generate_init_zsh_script();
        // When ** is not present, fall through to normal completion. The fallthrough
        // must be the non-dot widget form: `zle .expand-or-complete` invokes the raw
        // ZLE builtin, which bypasses the completion system and completes nothing in
        // any shell that has run compinit (#23).
        assert!(script.contains("zle expand-or-complete"));
        assert!(!script.contains("zle .expand-or-complete"));
    }

    #[test]
    #[serial]
    fn test_init_zsh_no_bind_variant_falls_through_to_completion_system() {
        std::env::set_var("SSHM_NO_BIND", "1");
        let script = generate_init_zsh_script();
        assert!(script.contains("zle expand-or-complete"));
        assert!(!script.contains("zle .expand-or-complete"));
        std::env::remove_var("SSHM_NO_BIND");
    }

    #[test]
    #[serial]
    fn test_init_zsh_completion_binds_tab_by_default() {
        std::env::remove_var("SSHM_NO_BIND");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_zsh_script();
        // Tab binding (^I) for the completion trigger
        assert!(script.contains("bindkey '^I' _sshm_completion_picker"));
    }

    #[test]
    #[serial]
    fn test_init_zsh_completion_binds_tab_suppressed_with_no_bind() {
        std::env::set_var("SSHM_NO_BIND", "1");

        let script = generate_init_zsh_script();
        assert!(!script.contains("bindkey '^I'"));

        std::env::remove_var("SSHM_NO_BIND");
    }

    #[test]
    fn test_init_zsh_completion_saves_buffer_before_stripping() {
        let script = generate_init_zsh_script();
        // The saved_buffer must capture the state BEFORE stripping **
        // so that cancellation restores the full original buffer
        let saved_before = script
            .find("local saved_buffer=\"$BUFFER\"")
            .expect("must save buffer");
        let strip_pos = script
            .find("local stripped=\"${LBUFFER%'**'}\"")
            .expect("must strip **");
        // saved_buffer must appear before stripping in the completion function
        assert!(
            saved_before < strip_pos,
            "saved_buffer must be captured before stripping **"
        );
    }

    #[test]
    fn test_init_zsh_completion_registers_widget() {
        let script = generate_init_zsh_script();
        assert!(script.contains("zle -N _sshm_completion_picker"));
    }

    #[test]
    fn test_init_zsh_completion_replaces_buffer() {
        let script = generate_init_zsh_script();
        // Result replaces the buffer (not appended) so the full ssh command is on the command line.
        assert!(script.contains("BUFFER=\"$result\""));
        assert!(script.contains("CURSOR=${#BUFFER}"));
    }

    // ── sshm init bash snapshot tests (AC7) ────────────────────────────────────────────────

    #[test]
    #[serial]
    fn test_snapshot_init_bash_default() {
        // Clear any env overrides
        std::env::remove_var("SSHM_NO_BIND");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_bash_script();
        insta::assert_snapshot!(script, @r###"
        # sshm init bash - Inline Picker Widget
        # Sourced via: eval "$(sshm init bash)"
        # Bind key: \e\C-s (override with SSHM_BIND_KEY)

        _sshm_inline_picker() {
            local result exit_code
            result=$(sshm pick --query "$READLINE_LINE")
            exit_code=$?
            if [[ $exit_code -ne 0 ]]; then
                return
            fi
            if [[ -n "$result" ]]; then
                READLINE_LINE="$result"
                READLINE_POINT=${#READLINE_LINE}
            fi
        }

        # Bind the widget to the trigger key (Ctrl+Alt+S)
        bind -x '"\e\C-s":_sshm_inline_picker'
        "###);
    }

    #[test]
    #[serial]
    fn test_snapshot_init_bash_no_bind() {
        // Set SSHM_NO_BIND to suppress bind lines
        std::env::set_var("SSHM_NO_BIND", "1");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_bash_script();
        insta::assert_snapshot!(script, @r###"
        # sshm init bash - Inline Picker Widget (no bind)
        # Sourced via: eval "$(sshm init bash)"
        # Note: Bind lines suppressed by SSHM_NO_BIND=1

        _sshm_inline_picker() {
            local result exit_code
            result=$(sshm pick --query "$READLINE_LINE")
            exit_code=$?
            if [[ $exit_code -ne 0 ]]; then
                return
            fi
            if [[ -n "$result" ]]; then
                READLINE_LINE="$result"
                READLINE_POINT=${#READLINE_LINE}
            fi
        }
        "###);

        // Clean up
        std::env::remove_var("SSHM_NO_BIND");
    }

    #[test]
    #[serial]
    fn test_snapshot_init_bash_custom_bind_key() {
        // Clear SSHM_NO_BIND and set custom SSHM_BIND_KEY
        std::env::remove_var("SSHM_NO_BIND");
        std::env::set_var("SSHM_BIND_KEY", "\\C-t");

        let script = generate_init_bash_script();
        insta::assert_snapshot!(script, @r###"
        # sshm init bash - Inline Picker Widget
        # Sourced via: eval "$(sshm init bash)"
        # Bind key: \C-t (override with SSHM_BIND_KEY)

        _sshm_inline_picker() {
            local result exit_code
            result=$(sshm pick --query "$READLINE_LINE")
            exit_code=$?
            if [[ $exit_code -ne 0 ]]; then
                return
            fi
            if [[ -n "$result" ]]; then
                READLINE_LINE="$result"
                READLINE_POINT=${#READLINE_LINE}
            fi
        }

        # Bind the widget to the trigger key (Ctrl+Alt+S)
        bind -x '"\C-t":_sshm_inline_picker'
        "###);

        // Clean up
        std::env::remove_var("SSHM_BIND_KEY");
    }

    // ── sshm init bash contract assertions (AC8) ────────────────────────────────────────────

    #[test]
    fn test_init_bash_contains_sshm_pick() {
        let script = generate_init_bash_script();
        assert!(script.contains("sshm pick"));
    }

    #[test]
    fn test_init_bash_seeds_query_readline_line() {
        let script = generate_init_bash_script();
        assert!(script.contains("--query \"$READLINE_LINE\""));
    }

    #[test]
    #[serial]
    fn test_init_bash_binds_ctrl_alt_s_by_default() {
        std::env::remove_var("SSHM_NO_BIND");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_bash_script();
        assert!(script.contains("bind -x"));
        // \e\C-s is bash notation for Ctrl+Alt+S.
        assert!(script.contains("\\e\\C-s"));
    }

    #[test]
    #[serial]
    fn test_init_bash_suppresses_bind_with_no_bind_flag() {
        std::env::set_var("SSHM_NO_BIND", "1");

        let script = generate_init_bash_script();
        assert!(!script.contains("bind -x"));
        assert!(script.contains("no bind"));

        // Clean up
        std::env::remove_var("SSHM_NO_BIND");
    }

    #[test]
    #[serial]
    fn test_init_bash_honors_sshm_bind_key() {
        std::env::remove_var("SSHM_NO_BIND");
        std::env::set_var("SSHM_BIND_KEY", "\\C-x");

        let script = generate_init_bash_script();
        assert!(script.contains("bind -x"));
        assert!(script.contains("\\C-x"));

        // Clean up
        std::env::remove_var("SSHM_BIND_KEY");
    }

    #[test]
    fn test_init_bash_splices_into_readline_line_with_cursor_to_end() {
        let script = generate_init_bash_script();
        assert!(script.contains("READLINE_LINE=\"$result\""));
        assert!(script.contains(r"READLINE_POINT=${#READLINE_LINE}"));
    }

    #[test]
    fn test_init_bash_leaves_readline_line_untouched_on_nonzero_exit() {
        let script = generate_init_bash_script();
        // On non-zero exit, the function returns without modifying READLINE_LINE.
        assert!(script.contains("if [[ $exit_code -ne 0 ]]; then"));
        assert!(script.contains("return"));
        // No assignment to READLINE_LINE on the cancel path.
        let normal_return_count = script.matches("return").count();
        assert!(normal_return_count >= 1);
    }

    // ── sshm init bash TAB-safety contract tests (#23) ─────────────────────────────────────

    #[test]
    #[serial]
    fn test_init_bash_never_rebinds_tab() {
        std::env::remove_var("SSHM_NO_BIND");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_bash_script();
        // bash cannot chain from a `bind -x` function back to normal completion,
        // so binding TAB kills completion for the whole session. The bash init
        // must never touch \C-i, and must not ship the picker function at all.
        assert!(!script.contains("\\C-i"));
        assert!(!script.contains("_sshm_completion_picker"));
    }

    #[test]
    #[serial]
    fn test_init_bash_no_bind_never_rebinds_tab() {
        std::env::set_var("SSHM_NO_BIND", "1");

        let script = generate_init_bash_script();
        assert!(!script.contains("\\C-i"));
        assert!(!script.contains("_sshm_completion_picker"));

        std::env::remove_var("SSHM_NO_BIND");
    }

    #[test]
    #[serial]
    fn test_init_bash_custom_key_still_never_rebinds_tab() {
        std::env::remove_var("SSHM_NO_BIND");
        std::env::set_var("SSHM_BIND_KEY", "\\C-t");

        let script = generate_init_bash_script();
        assert!(!script.contains("\\C-i"));
        assert!(!script.contains("_sshm_completion_picker"));
        // The custom key still binds the inline picker.
        assert!(script.contains(r###"bind -x '"\C-t":_sshm_inline_picker'"###));

        std::env::remove_var("SSHM_BIND_KEY");
    }
}
