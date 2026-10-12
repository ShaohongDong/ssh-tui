pub mod app;
pub mod askpass;
pub mod config;
pub mod credentials;
pub mod i18n;
pub mod input;
pub mod proxy;
pub mod session;
pub mod ssh;
pub mod terminal;
pub mod theme;
pub mod ui;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("ssh-tui supports Linux and macOS only");
