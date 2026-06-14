//! The app's identity — its name and slug — referenced wherever the program
//! announces itself to the OS (autostart entry, registry key, tray, config dir,
//! D-Bus/KWin script names). One definition so the strings aren't re-typed.

/// Human-facing display name (desktop entry `Name`, Windows registry value, tray).
pub const APP_NAME: &str = "WinAlpha";
/// Lowercase slug for filenames and identifiers (autostart `.desktop`, config dir).
pub const APP_ID: &str = "winalpha";
/// Reverse-DNS qualifier + organisation for `directories::ProjectDirs`. Only
/// affects the macOS/Windows config path (Linux uses [`APP_ID`] alone); kept here
/// so the identity isn't hardcoded inline at the lookup site.
pub const APP_QUALIFIER: &str = "com";
pub const APP_ORG: &str = "windowtransparency";
