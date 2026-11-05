mod client;
mod commands;
mod model;
mod server;

pub use client::run_cli;
pub use server::start;

use std::path::PathBuf;

fn socket_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home).join(".cache/tessel-editor/ui.sock"))
}
