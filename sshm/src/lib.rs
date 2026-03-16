pub mod app;
pub mod config;
pub mod ssh;
pub mod update;

pub use ssh::{build_ssh_args, execute_ssh};

#[cfg(test)]
mod main_rs {}
