//! Boost mode: while a game launched from XBGG runs, free resources for it —
//! freeze chosen background apps and release their RAM, switch Windows to
//! "Best performance", optionally raise the game's priority — and undo all of
//! it when the game closes. Every change is journaled first, so it can be
//! undone even after a crash.

#[cfg(windows)]
pub mod apps;
pub mod config;
pub mod journal;
#[cfg(windows)]
pub mod services;
#[cfg(windows)]
pub mod session;
#[cfg(windows)]
pub mod sys;

pub use config::BoostConfig;
