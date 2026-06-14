//! Diagnostic logging via the `tracing` crate, active only in debug builds.
//!
//! In release builds every `tracing` macro is compiled out at the source level
//! (the `release_max_level_off` feature on `tracing` in `Cargo.toml`) and
//! [`init`] is an empty no-op, so logging contributes nothing to the
//! size-optimized release binary.

/// Install the global tracing subscriber: a human-readable stdout logger in
/// debug builds. The level defaults to `TRACE` but is overridable via `RUST_LOG`
/// (e.g. `RUST_LOG=win_alpha=debug`). Does nothing in release builds.
pub fn init() {
    #[cfg(debug_assertions)]
    {
        use std::str::FromStr;
        use tracing::Level;
        use tracing_subscriber::{filter::Targets, fmt, prelude::*};

        // `Targets` parses the same `target=level` syntax as `RUST_LOG` without
        // pulling in `env-filter`'s `regex`/`matchers` dependencies.
        let filter = std::env::var("RUST_LOG")
            .ok()
            .and_then(|raw| Targets::from_str(&raw).ok())
            .unwrap_or_else(|| Targets::new().with_default(Level::TRACE));

        tracing_subscriber::registry()
            .with(fmt::layer().with_writer(std::io::stdout).with_filter(filter))
            .init();
    }
}
