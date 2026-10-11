use sshm_e2e::fixture::{seeded_fixture, sshm_binary};
use sshm_e2e::Pty;
use std::time::Duration;

fn dump(pty: &mut Pty, what: &str) {
    println!("######## {what}");
    for (i, line) in pty.visible_rows().iter().enumerate() {
        if !line.trim().is_empty() {
            println!("{i:2}|{line}|");
        }
    }
}

#[test]
fn esc_paths() {
    let fixture = seeded_fixture(&["web-01", "db-01"]);
    let mut pty = Pty::spawn(sshm_binary(), &["manage"], &fixture).unwrap();
    pty.wait_for_frame().unwrap();
    pty.key(b"\x05");
    pty.wait_for_text("Edit web-01").unwrap();
    pty.key(b"\x1b");
    std::thread::sleep(Duration::from_millis(150));
    dump(&mut pty, "esc straight from edit");

    pty.key(b"\x05");
    pty.wait_for_text("Edit web-01").unwrap();
    pty.key(b"\x1b[B");
    std::thread::sleep(Duration::from_millis(60));
    pty.key(b"\x1b[B");
    std::thread::sleep(Duration::from_millis(60));
    pty.key(b"\x1b[A");
    std::thread::sleep(Duration::from_millis(60));
    dump(&mut pty, "down down up");
    pty.key(b"\r");
    std::thread::sleep(Duration::from_millis(60));
    dump(&mut pty, "after enter");
    pty.key(b"\x1b");
    std::thread::sleep(Duration::from_millis(150));
    dump(&mut pty, "after esc");
}
