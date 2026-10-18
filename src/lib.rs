#![forbid(unsafe_op_in_unsafe_fn)]
#[cfg(not(unix))]
compile_error!("TERYON 0.1 supports macOS and Linux. Use WSL on Windows.");
pub mod brand;
pub mod config;
pub mod daemon;
pub mod dashboard;
#[cfg(feature = "desktop")]
pub mod desktop;
pub mod ipc;
pub mod keymap;
pub mod keys;
pub mod protocol;
pub mod record;
pub mod settings;
pub mod theme;
pub mod transport;
pub mod ui;
pub mod uikit;
pub mod util;
