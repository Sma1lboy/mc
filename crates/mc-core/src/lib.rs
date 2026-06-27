//! `mc-core` — the UI-free Minecraft launcher engine.
//!
//! Everything the launcher does (resolve versions, download & verify files,
//! authenticate, find/install Java, build the command line, spawn the game)
//! lives here. It has no knowledge of Tauri or any UI; the CLI and desktop
//! shells are thin consumers. See `docs/04-rust-tauri-design.md`.

pub mod account;
pub mod auth;
pub mod diagnostics;
pub mod download;
pub mod error;
pub mod friend;
pub mod fs;
pub mod instance;
pub mod java;
pub mod launch;
pub mod loader;
pub mod meta;
pub mod modpack;
pub mod modplatform;
pub mod paths;
pub mod realm;
pub mod server;
pub mod settings;
pub mod version;

pub use error::{CoreError, Result};
pub use mc_types as types;

/// Launcher identity reported to the game (`${launcher_name}` / `_version`).
pub const LAUNCHER_NAME: &str = "mc-launcher";
pub const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");
