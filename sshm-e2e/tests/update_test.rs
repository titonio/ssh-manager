//! The Update Note, served by a release server on the loopback interface.
//!
//! The note is the only thing in the frame that reports a fact the machine does
//! not own — what GitHub calls the latest release — and it is drawn from a cache
//! a *previous* run wrote, so the interesting behaviour is spread across two
//! process launches and one socket. Behind the in-crate seam the checker is a
//! function pointer, which is exactly the seam that lets the real one, with its
//! real HTTP client and its real cache file, go untested.
//!
//! Everything here is loopback. The stub binds `127.0.0.1:0`, the fixture points
//! `SSHM_GITHUB_API_URL` at it, and the test asserts afterwards that the stub is
//! the only thing that was asked. Nothing in this suite can reach the real
//! release API, and nothing in it can replace the binary it is running.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::thread;

use sshm_e2e::fixture::{seeded_fixture, sshm_binary};
use sshm_e2e::{Fixture, Pty, DEFAULT_TIMEOUT};

/// The version the stub claims is the latest.
///
/// Higher than anything sshm will plausibly reach, so the "is it newer" rule is
/// exercised rather than skipped, and unmistakably not a real release.
const NEWER: &str = "9.9.9";

/// A release API on the loopback interface, answering the one question the
/// update check asks.
///
/// Hand-rolled rather than pulled in as a dependency: the check asks for one URL
/// and reads one JSON body, and a test framework that has to be kept compatible
/// with the thing it is testing is a second thing to debug. Every request line is
/// recorded, because "we only talked to this" is a claim that needs evidence.
struct ReleaseStub {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
}

impl ReleaseStub {
    fn serve() -> ReleaseStub {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let addr = listener.local_addr().expect("the bound address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&requests);

        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 2048];
                let read = stream.read(&mut buf).unwrap_or(0);
                let request_line = String::from_utf8_lossy(&buf[..read])
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_string();
                shared.lock().expect("the log").push(request_line);

                // The asset name is the platform's own, spelled from the same
                // facts the release process uses: the checker will not call a
                // release the latest unless it can find something to install
                // for the machine asking.
                let asset = format!("sshm-{}-unknown-linux-musl.tar.gz", std::env::consts::ARCH);
                let body = format!(
                    r#"{{"tag_name":"v{NEWER}","name":"v{NEWER}","created_at":"2024-01-01T00:00:00Z","draft":false,"prerelease":false,"assets":[{{"name":"{asset}","url":"http://{addr}/asset"}}]}}"#
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        ReleaseStub { addr, requests }
    }

    /// What to hand the app as `SSHM_GITHUB_API_URL`.
    fn api_root(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The request lines it was sent, in order.
    fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("the log").clone()
    }
}

/// A fixture whose release API is the stub, and nothing else.
fn stubbed(stub: &ReleaseStub) -> Fixture {
    seeded_fixture(&["web-01", "db-01"]).with_env("SSHM_GITHUB_API_URL", stub.api_root())
}

/// Ask, then look: the check caches, the frame reads the cache.
///
/// Two launches on purpose. The frame's paint path never touches the network —
/// that is the design, and it is what keeps first paint instant — so the note can
/// only appear because a previous run wrote the answer down. A test that faked
/// the cache file would prove the frame can draw a string; this proves the
/// pipeline that puts a real answer in front of it.
#[test]
fn a_newer_release_on_the_loopback_api_is_drawn_as_a_note() {
    let stub = ReleaseStub::serve();
    let fixture = stubbed(&stub);
    let binary = sshm_binary();
    let before = std::fs::metadata(&binary).expect("the binary is there");

    // **Check for Updates** — the question, over the seam, into the cache.
    let mut check = Pty::spawn(&binary, &["check-update"], &fixture).expect("run check-update");
    check
        .wait_until("the answer on stdout", DEFAULT_TIMEOUT, |screen| {
            screen
                .contents()
                .contains(&format!("Update available: v{NEWER}"))
        })
        .expect("the check reports the release the stub served");
    check
        .wait_exit(DEFAULT_TIMEOUT)
        .expect("check-update exits")
        .assert_code(0);

    // The frame, in a second process, with no network of its own to ask.
    let mut pty = Pty::spawn(&binary, &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_text(&format!(
        "\u{25c6} update available: v{NEWER} \u{2014} run sshm update"
    ))
    .expect("the note names the version the stub served and the command to run");

    // Loopback only, and only the one question the seam defines.
    let asked = stub.requests();
    assert_eq!(
        asked.len(),
        1,
        "the frame's paint path must not go looking for a release itself"
    );
    assert_eq!(
        asked[0], "GET /repos/titonio/ssh-manager/releases/latest HTTP/1.1",
        "the API-root seam is what was used, and nothing else was asked"
    );

    // And nothing replaced the binary it is running. The note tells the user to
    // run a command; it does not run it for them, and Apply Update is a different
    // command that this suite never invokes.
    let after = std::fs::metadata(&binary).expect("the binary is still there");
    assert_eq!(
        after.len(),
        before.len(),
        "asking about a release must not touch the binary"
    );
    assert_eq!(
        after.modified().expect("a modification time"),
        before.modified().expect("a modification time"),
        "asking about a release must not rewrite the binary either"
    );
}

/// A note is drawn above a list that still works.
///
/// The note is one line above the rows, and the rows are the reason the frame is
/// open. A note that stole the keystrokes, or pushed the rows out of the frame,
/// would be a regression nobody would notice until they could not select
/// anything — which is the class of thing a golden of a frame with a note in it
/// would have caught, and the reason this checks the interaction rather than the
/// wording again.
#[test]
fn the_note_sits_above_a_list_that_still_works() {
    let stub = ReleaseStub::serve();
    let fixture = stubbed(&stub);
    let binary = sshm_binary();

    let mut check = Pty::spawn(&binary, &["check-update"], &fixture).expect("run check-update");
    check
        .wait_exit(DEFAULT_TIMEOUT)
        .expect("check-update exits");

    let mut pty = Pty::spawn(&binary, &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_text("update available")
        .expect("the note is drawn");
    pty.wait_for_text("web-01").expect("the list is drawn");

    // The cursor is still the cursor, and it still moves.
    pty.key(b"\x1b[B");
    pty.wait_until("the cursor to move", DEFAULT_TIMEOUT, |screen| {
        screen.contents().contains("\u{276f} db-01")
    })
    .expect("the note does not take the frame's keys away");

    pty.key(b"\x1b");
    pty.wait_exit(DEFAULT_TIMEOUT)
        .expect("the frame closes")
        .assert_code(130);
}
