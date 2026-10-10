pub mod app;
pub mod config;
pub mod i18n;
pub mod input;
pub mod proxy;
pub mod session;
pub mod ssh;
pub mod terminal;
pub mod theme;
pub mod ui;
#[cfg(not(target_os = "linux"))]
compile_error!("ssh-tui 0.1 supports Linux only");
