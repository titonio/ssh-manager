//! End-to-end tests for `sshm`, run against the real binary in a real terminal.
//!
//! Everything in the `sshm` crate's own suite drives the frame through a seam —
//! a fake `Events`, a `Vec<u8>` standing in for a terminal — which is what makes
//! the keystroke rules and the painting testable at all. The seam is also a
//! blind spot: a chord that stops arriving as the bytes crossterm decodes, a
//! form that stops persisting, a shell binding that stops binding, all stay
//! green behind it.
//!
//! This crate has no seam. It builds the binary, spawns it in a pseudo-terminal
//! with a throwaway `HOME`, presses keys as bytes, and reads the answer off the
//! screen the terminal emulator parsed. It lives outside the `sshm` crate on
//! purpose: `cargo test` in `sshm/` never sees it, so the mutation sweep, the
//! coverage floor and the CRAP gate keep running exactly what they run today
//! instead of running this several hundred times over.
//!
//! Two modules, and the split is the whole design:
//!
//! * [`pty`] — the terminal. Spawning, keystrokes in, screen grid out, waits
//!   with deadlines, termios checks, cleanup.
//! * [`fixture`] — the world around it. A temp `HOME`, a seeded store, the
//!   `ssh` stub, the environment a child is born into.

pub mod colour;
pub mod fixture;
pub mod pty;

pub use colour::token;
pub use fixture::Fixture;
pub use pty::{Pty, DEFAULT_TIMEOUT};
