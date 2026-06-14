//! Application layer: the runtime [`state`], its persisted [`config`], and the
//! user-facing surfaces — the [`tray`] menu and [`ui`] window plumbing. The
//! lower layers (`platform` and `transparency`) sit outside this module.

pub mod config;
pub mod state;
pub mod theme;
pub mod tray;
pub mod ui;
