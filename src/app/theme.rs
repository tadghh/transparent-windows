//! System color-scheme detection for Slint's `Palette`.
//!
//! Slint's public Rust API doesn't expose the platform-detected color scheme
//! (`Window::color_scheme()` is `pub(crate)`), so we read it directly from the
//! OS and push it into `Palette.color-scheme`. With that explicitly set, the
//! std-widgets pick the matching light/dark color set.

use slint::language::ColorScheme;

/// Best-effort read of the user's system preference. Returns
/// [`ColorScheme::Unknown`] when no signal is available, letting Slint fall
/// back to its (light) default.
pub fn detect() -> ColorScheme {
    #[cfg(windows)]
    {
        detect_windows()
    }
    #[cfg(unix)]
    {
        detect_unix()
    }
}

#[cfg(windows)]
fn detect_windows() -> ColorScheme {
    use windows::{
        Win32::{
            Foundation::ERROR_SUCCESS,
            System::Registry::{
                HKEY, HKEY_CURRENT_USER, KEY_READ, RegCloseKey, RegOpenKeyExA, RegQueryValueExA,
            },
        },
        core::PCSTR,
    };

    // HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\AppsUseLightTheme
    // is a REG_DWORD: 0 = dark, 1 = light. Absent on older builds → Unknown.
    let subkey = b"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0";
    let value_name = b"AppsUseLightTheme\0";

    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExA(
            HKEY_CURRENT_USER,
            PCSTR::from_raw(subkey.as_ptr()),
            Some(0),
            KEY_READ,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return ColorScheme::Unknown;
        }

        let mut data: u32 = 0;
        let mut size: u32 = core::mem::size_of::<u32>() as u32;
        let status = RegQueryValueExA(
            key,
            PCSTR::from_raw(value_name.as_ptr()),
            None,
            None,
            Some(&mut data as *mut u32 as *mut u8),
            Some(&mut size),
        );
        let _ = RegCloseKey(key);

        if status != ERROR_SUCCESS {
            return ColorScheme::Unknown;
        }
        if data == 0 {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        }
    }
}

#[cfg(unix)]
fn detect_unix() -> ColorScheme {
    // Prefer the freedesktop standard (XDG color-scheme), surfaced by GNOME ≥42,
    // KDE Plasma ≥5.26, and most modern DEs through `gsettings`. Output is one of
    //   'default' | 'prefer-dark' | 'prefer-light'
    // wrapped in single quotes.
    if let Ok(output) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
        && output.status.success()
    {
        let value = String::from_utf8_lossy(&output.stdout);
        let trimmed = value.trim().trim_matches('\'');
        match trimmed {
            "prefer-dark" => return ColorScheme::Dark,
            "prefer-light" | "default" => return ColorScheme::Light,
            _ => {}
        }
    }

    // KDE exposes the same preference via `kreadconfig5`/`kreadconfig6` against
    // kdeglobals → [General] → ColorScheme (e.g. "BreezeDark"). Try both.
    for bin in ["kreadconfig6", "kreadconfig5"] {
        if let Ok(output) = std::process::Command::new(bin)
            .args(["--group", "General", "--key", "ColorScheme"])
            .output()
            && output.status.success()
        {
            let value = String::from_utf8_lossy(&output.stdout)
                .trim()
                .to_ascii_lowercase();
            if value.contains("dark") {
                return ColorScheme::Dark;
            }
            if !value.is_empty() {
                return ColorScheme::Light;
            }
        }
    }

    // Last resort: GTK_THEME often ends in `:dark` when the user forced dark.
    if let Ok(theme) = std::env::var("GTK_THEME")
        && theme.to_ascii_lowercase().contains("dark")
    {
        return ColorScheme::Dark;
    }

    ColorScheme::Unknown
}
