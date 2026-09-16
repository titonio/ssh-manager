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
use update::UpdateResult;

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

fn main() -> io::Result<()> {
    run_main(
        run_frame_command,
        update::force_check_for_update,
        update::cached_update,
    )
}

fn run_main(
    run_frame_fn: fn(Emit, &mut dyn connections::Store, String, Option<String>) -> io::Result<()>,
    check_update_fn: fn() -> UpdateResult,
    read_note_fn: fn() -> Option<String>,
) -> io::Result<()> {
    let cli = Cli::parse();
    dispatch(cli, run_frame_fn, check_update_fn, read_note_fn)
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
fn dispatch(
    cli: Cli,
    run_frame_fn: fn(Emit, &mut dyn connections::Store, String, Option<String>) -> io::Result<()>,
    check_update_fn: fn() -> UpdateResult,
    read_note_fn: fn() -> Option<String>,
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

    // `sshm pick` — the insert emit. Runs before the update checker so a
    // captured stdout is never polluted by it, and returns straight from
    // the frame. The note is read from cache here, once, before the frame
    // opens (#39).
    if let Some(Commands::Pick { query }) = cli.command {
        let mut config = config::Config::load();
        let note = read_note_fn();
        return run_frame_fn(Emit::Insert, &mut config, query.unwrap_or_default(), note);
    }

    // `sshm manage` — the edit emit, in the manage frame.
    if let Some(Commands::Manage) = cli.command {
        let mut config = config::Config::load();
        let note = read_note_fn();
        return run_frame_fn(Emit::Edit, &mut config, String::new(), note);
    }

    // Handle check-update flag or command
    if cli.check_update || matches!(cli.command, Some(Commands::CheckUpdate)) {
        match check_update_fn() {
            UpdateResult::UpdateAvailable { version } => {
                println!("Update available: v{}", version);
                println!("Run again without flag to update automatically.");
                return Ok(());
            }
            UpdateResult::NoUpdate => {
                println!("No update available.");
                return Ok(());
            }
            UpdateResult::Error(e) => {
                eprintln!("Error checking for updates: {}", e);
            }
        }
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
    // through to: the frame is the whole surface now. The note is read
    // from cache here, once, before the frame opens (#39).
    let mut config = config::Config::load();
    let note = read_note_fn();
    run_frame_fn(Emit::Execute, &mut config, String::new(), note)
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
        zle .expand-or-complete
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
        zle .expand-or-complete
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

/// Generate the bash initialization script for the inline picker widget and
/// **<TAB> completion trigger.
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
        r###"# sshm init bash - Inline Picker Widget + **<TAB> completion trigger (no bind)
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

# **<TAB> completion trigger (bash): when READLINE_LINE ends with **,
# pressing Tab opens the inline picker. Note: without **, Tab has no
# effect (bash cannot chain to normal completion from a key-bound function).
_sshm_completion_picker() {
    if [[ "$READLINE_LINE" == *'**' ]]; then
        local saved="$READLINE_LINE"
        local stripped="${READLINE_LINE%'**'}"
        local result exit_code
        result=$(sshm pick --query "$stripped")
        exit_code=$?
        if [[ $exit_code -ne 0 ]]; then
            READLINE_LINE="$saved"
            return
        fi
        if [[ -n "$result" ]]; then
            READLINE_LINE="$result"
            READLINE_POINT=${#READLINE_LINE}
        fi
    fi
}
"###
        .to_string()
    } else {
        format!(
            r###"# sshm init bash - Inline Picker Widget + **<TAB> completion trigger
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

# **<TAB> completion trigger (bash): when READLINE_LINE ends with **,
# pressing Tab opens the inline picker. Note: without **, Tab has no
# effect (bash cannot chain to normal completion from a key-bound function).
_sshm_completion_picker() {{
    if [[ "$READLINE_LINE" == *'**' ]]; then
        local saved="$READLINE_LINE"
        local stripped="${{READLINE_LINE%'**'}}"
        local result exit_code
        result=$(sshm pick --query "$stripped")
        exit_code=$?
        if [[ $exit_code -ne 0 ]]; then
            READLINE_LINE="$saved"
            return
        fi
        if [[ -n "$result" ]]; then
            READLINE_LINE="$result"
            READLINE_POINT=${{#READLINE_LINE}}
        fi
    fi
}}

# Bind the widget to the trigger key (Ctrl+Alt+S)
bind -x '"{bind_key}":_sshm_inline_picker'

# Bind **<TAB> completion trigger (Tab key)
bind -x '"\C-i":_sshm_completion_picker'
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
        dispatch(cli, record_frame, check_update_must_not_run, no_cached_note).unwrap();
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
        dispatch(cli, record_frame, check_update_must_not_run, no_cached_note).unwrap();
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
        dispatch(cli, record_frame, check_update_must_not_run, no_cached_note).unwrap();
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
        dispatch(cli, record_frame, check_update_must_not_run, no_cached_note).unwrap();
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
        dispatch(cli, record_frame, check_update_must_not_run, no_cached_note).unwrap();
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
        dispatch(cli, record_frame, check_update_must_not_run, no_cached_note).unwrap();
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
        dispatch(cli, record_frame, no_update, no_cached_note).unwrap();
        assert!(take_calls().is_empty(), "-c checks, it does not frame");
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
                zle .expand-or-complete
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
                zle .expand-or-complete
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
                zle .expand-or-complete
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
        // When ** is not present, fall through to normal completion
        assert!(script.contains("zle .expand-or-complete"));
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
        # sshm init bash - Inline Picker Widget + **<TAB> completion trigger
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

        # **<TAB> completion trigger (bash): when READLINE_LINE ends with **,
        # pressing Tab opens the inline picker. Note: without **, Tab has no
        # effect (bash cannot chain to normal completion from a key-bound function).
        _sshm_completion_picker() {
            if [[ "$READLINE_LINE" == *'**' ]]; then
                local saved="$READLINE_LINE"
                local stripped="${READLINE_LINE%'**'}"
                local result exit_code
                result=$(sshm pick --query "$stripped")
                exit_code=$?
                if [[ $exit_code -ne 0 ]]; then
                    READLINE_LINE="$saved"
                    return
                fi
                if [[ -n "$result" ]]; then
                    READLINE_LINE="$result"
                    READLINE_POINT=${#READLINE_LINE}
                fi
            fi
        }

        # Bind the widget to the trigger key (Ctrl+Alt+S)
        bind -x '"\e\C-s":_sshm_inline_picker'

        # Bind **<TAB> completion trigger (Tab key)
        bind -x '"\C-i":_sshm_completion_picker'
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
        # sshm init bash - Inline Picker Widget + **<TAB> completion trigger (no bind)
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

        # **<TAB> completion trigger (bash): when READLINE_LINE ends with **,
        # pressing Tab opens the inline picker. Note: without **, Tab has no
        # effect (bash cannot chain to normal completion from a key-bound function).
        _sshm_completion_picker() {
            if [[ "$READLINE_LINE" == *'**' ]]; then
                local saved="$READLINE_LINE"
                local stripped="${READLINE_LINE%'**'}"
                local result exit_code
                result=$(sshm pick --query "$stripped")
                exit_code=$?
                if [[ $exit_code -ne 0 ]]; then
                    READLINE_LINE="$saved"
                    return
                fi
                if [[ -n "$result" ]]; then
                    READLINE_LINE="$result"
                    READLINE_POINT=${#READLINE_LINE}
                fi
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
        # sshm init bash - Inline Picker Widget + **<TAB> completion trigger
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

        # **<TAB> completion trigger (bash): when READLINE_LINE ends with **,
        # pressing Tab opens the inline picker. Note: without **, Tab has no
        # effect (bash cannot chain to normal completion from a key-bound function).
        _sshm_completion_picker() {
            if [[ "$READLINE_LINE" == *'**' ]]; then
                local saved="$READLINE_LINE"
                local stripped="${READLINE_LINE%'**'}"
                local result exit_code
                result=$(sshm pick --query "$stripped")
                exit_code=$?
                if [[ $exit_code -ne 0 ]]; then
                    READLINE_LINE="$saved"
                    return
                fi
                if [[ -n "$result" ]]; then
                    READLINE_LINE="$result"
                    READLINE_POINT=${#READLINE_LINE}
                fi
            fi
        }

        # Bind the widget to the trigger key (Ctrl+Alt+S)
        bind -x '"\C-t":_sshm_inline_picker'

        # Bind **<TAB> completion trigger (Tab key)
        bind -x '"\C-i":_sshm_completion_picker'
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

    // ── sshm init bash **<TAB> completion trigger contract tests ───────────────────────────

    #[test]
    fn test_init_bash_has_completion_trigger_function() {
        let script = generate_init_bash_script();
        assert!(script.contains("_sshm_completion_picker"));
    }

    #[test]
    fn test_init_bash_completion_checks_readline_for_doublestar() {
        let script = generate_init_bash_script();
        // The widget checks if READLINE_LINE ends with ** ($READLINE_LINE is quoted)
        assert!(script.contains("\"$READLINE_LINE\" == *'**'"));
    }

    #[test]
    fn test_init_bash_completion_strips_doublestar() {
        let script = generate_init_bash_script();
        assert!(script.contains("${READLINE_LINE%'**'}"));
    }

    #[test]
    #[serial]
    fn test_init_bash_completion_binds_tab_by_default() {
        std::env::remove_var("SSHM_NO_BIND");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_bash_script();
        assert!(script.contains(r###"bind -x '"\C-i":_sshm_completion_picker'"###));
    }

    #[test]
    #[serial]
    fn test_init_bash_completion_binds_tab_suppressed_with_no_bind() {
        std::env::set_var("SSHM_NO_BIND", "1");

        let script = generate_init_bash_script();
        assert!(!script.contains(r###"\C-i":_sshm_completion_picker"###));

        std::env::remove_var("SSHM_NO_BIND");
    }

    #[test]
    fn test_init_bash_completion_saves_buffer_before_stripping() {
        let script = generate_init_bash_script();
        let saved_before = script
            .find("local saved=\"$READLINE_LINE\"")
            .expect("must save buffer");
        let strip_pos = script.find("${READLINE_LINE%'**'}").expect("must strip **");
        assert!(
            saved_before < strip_pos,
            "saved must be captured before stripping **"
        );
    }

    #[test]
    fn test_init_bash_completion_seeds_query_with_stripped_line() {
        let script = generate_init_bash_script();
        assert!(script.contains("--query \"$stripped\""));
    }
}
