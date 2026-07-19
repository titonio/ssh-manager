pub mod app;
pub mod config;
pub mod picker;
pub mod ssh;
pub mod update;

pub use ssh::{build_ssh_args, execute_ssh};
pub use picker::{build_ssh_command, PickerOutcome};

#[cfg(test)]
mod main_rs {}
