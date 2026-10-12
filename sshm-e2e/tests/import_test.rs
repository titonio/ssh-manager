//! The first run on a machine that already has a `~/.ssh/config`.
//!
//! This file used to test the offer to import the whole config on first run.
//! That offer is gone, and the live merge is why: its premise was that an empty
//! store means an empty list, and a store with nothing in it now still lists
//! every host the config holds. Nothing is copied by looking, so there is
//! nothing left to offer.
//!
//! What the offer tests were protecting is still worth protecting, so it is
//! asserted here against the merged list instead: a first run opens on a list
//! and not on a prompt, and looking at the config writes nothing.

use sshm_e2e::fixture::{sshm_binary, Fixture};
use sshm_e2e::{Pty, DEFAULT_TIMEOUT};

/// A config with two usable stanzas in it and nothing else: a machine where
/// `ssh` already works and `sshm` has never been run.
///
/// Two, not one, because a first run that listed only the first host it found
/// would otherwise look like a first run that listed them all.
const CONFIG: &str = "Host web-01\n  HostName 10.0.0.1\n  User deploy\n\nHost db-01\n  HostName 10.0.0.2\n  User deploy\n";

fn fresh_machine() -> Fixture {
    Fixture::new()
        .with_ssh_config(CONFIG)
        .expect("the config is written")
}

/// A first run opens on the config's hosts and asks for nothing.
///
/// Three facts, none of them visible from inside the crate. The hosts the user
/// already has are on the very first screen, so the tool is useful before its
/// first keystroke. No prompt stands in front of them — an offer to copy them
/// would be a question about something the frame already does by looking, and a
/// first screen that demands an answer is the thing users resent. And the store
/// file is still absent afterwards, because listing a config host must not
/// quietly adopt it: the store is what the user chose, not what was found.
#[test]
fn a_first_run_opens_on_the_configs_hosts_and_asks_for_nothing() {
    let fixture = fresh_machine();
    let mut pty = Pty::spawn(sshm_binary(), &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_text("Manage Connections")
        .expect("the list is the first screen, not a question");
    pty.wait_until("both stanzas on the list", DEFAULT_TIMEOUT, |screen| {
        let text = screen.contents();
        text.contains("[ssh-config] web-01") && text.contains("[ssh-config] db-01")
    })
    .expect("both stanzas are listed, each tagged as a config row");

    let grid = pty.grid();
    assert!(
        grid.contains("[ssh-config] web-01 (deploy@10.0.0.1:22)")
            && grid.contains("[ssh-config] db-01 (deploy@10.0.0.2:22)"),
        "each row names the target its stanza describes\n{grid}"
    );
    assert!(
        !grid.contains("Import"),
        "nothing is offered, because nothing needs to be\n{grid}"
    );

    pty.key(b"\x1b");
    pty.wait_exit(DEFAULT_TIMEOUT)
        .expect("the frame closes")
        .assert_code(130);

    assert!(
        !fixture.store_exists(),
        "opening the frame on a config host must not write it into the store"
    );
}
