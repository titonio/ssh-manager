//! Visual harness for the #34 inline mechanics — not a product command.
//!
//! The three command runners are what will own `sshm` / `sshm pick` /
//! `sshm manage` (#35), and the `--emit` axis that decides what Enter means.
//! None of that exists yet, so this is how the frame gets looked at in a real
//! terminal — which is the only way this repo accepts a UI change as verified.
//!
//! ```text
//! cargo run --example inline_demo -- [pick|manage] [initial query]
//! ```
//!
//! The outcome is reported on stderr, off the UI stream: the product path will
//! route it per its emit mode instead.

use sshm::config::Connection;
use sshm::connections::Ephemeral;
use sshm::frame::FrameMode;
use sshm::inline::run_inline;

/// A fixture, not the user's config: the window only proves anything when
/// there are more rows than fit in it — and one of those rows is written in
/// two-column glyphs, because a wide alias is the case that breaks any
/// row-count that was measured in characters.
fn fixture() -> Vec<Connection> {
    let rows: [(&str, &str, &str, u16, Option<&str>); 15] = [
        ("web-01", "10.0.0.4", "deploy", 22, Some("prod")),
        ("web-02", "10.0.0.5", "deploy", 22, Some("prod")),
        ("db-primary", "10.0.1.10", "postgres", 5432, Some("prod")),
        ("db-replica", "10.0.1.11", "postgres", 5432, Some("prod")),
        (
            "staging-api",
            "staging.api.internal",
            "dev",
            22,
            Some("staging"),
        ),
        (
            "staging-worker",
            "staging.worker.internal",
            "dev",
            22,
            Some("staging"),
        ),
        ("bastion", "jumpbox.corp", "root", 22, None),
        ("nas-01", "192.168.1.50", "backup", 22, Some("home")),
        ("ci-runner-1", "ci-1.internal", "runner", 22, Some("ci")),
        ("ci-runner-2", "ci-2.internal", "runner", 22, Some("ci")),
        ("grafana", "metrics.internal", "admin", 22, Some("obs")),
        ("loki", "logs.internal", "admin", 22, Some("obs")),
        ("dev-box", "localhost", "dev", 2222, None),
        ("edge-cache", "cdn.edge", "ops", 22, Some("prod")),
        // Two columns per glyph: the row-count invariant's worst customer.
        ("服务器-01", "10.9.9.9", "ops", 22, Some("生产")),
    ];

    rows.into_iter()
        .enumerate()
        .map(|(i, (alias, host, user, port, folder))| Connection {
            id: format!("{i}"),
            alias: alias.to_string(),
            host: host.to_string(),
            user: user.to_string(),
            port,
            key_path: None,
            folder: folder.map(str::to_string),
        })
        .collect()
}

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let mode = match args.next().as_deref() {
        Some("manage") => FrameMode::Manage,
        _ => FrameMode::Pick,
    };
    let query = args.next().unwrap_or_default();
    // The demo deletes for real — off the fixture, never the user's
    // connections.json. That is what `Ephemeral` is for: the row the frame
    // drops under `y` is genuinely gone from the set being shown, and only
    // the durability is absent.
    let mut store = Ephemeral::new(fixture());

    let mut out = std::io::stdout();
    let outcome = run_inline(&mut out, &mut store, query, mode)?;

    eprintln!("[demo] {outcome:?}");
    Ok(())
}
