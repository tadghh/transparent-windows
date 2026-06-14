use super::OperatingSystem;
use anyhow::{Result, anyhow};
use std::path::PathBuf;

/// Linux host-OS integration (the [`OperatingSystem`] contract). Shared by both
/// the X11 and KWin window backends, which differ only in windowing.
pub struct Linux;

impl OperatingSystem for Linux {
    fn process_name_from_pid(pid: u32) -> Result<String> {
        if let Ok(name) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_owned());
            }
        }

        let exe = std::fs::read_link(format!("/proc/{pid}/exe"))?;
        exe.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_owned())
            .ok_or_else(|| anyhow!("Failed to resolve process name for pid {}", pid))
    }

    fn set_autostart(enabled: bool) -> Result<()> {
        let path = autostart_path()?;

        if enabled {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // A bare XDG autostart entry: dropping this in ~/.config/autostart is
            // the freedesktop spec honoured by all XDG desktops (GNOME, KDE, XFCE,
            // ...). Type/Name/Exec is the full minimal valid entry — its presence
            // (Hidden defaults to false) is what enables autostart, so no
            // DE-specific keys are needed.
            let exe = std::env::current_exe()?;
            let contents = format!(
                "[Desktop Entry]\nType=Application\nName={}\nExec={}\n",
                crate::identity::APP_NAME,
                exe.display()
            );
            std::fs::write(&path, contents)?;
        } else {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }

        Ok(())
    }

    fn get_autostart_state() -> bool {
        autostart_path().map(|path| path.exists()).unwrap_or(false)
    }

    fn open_path(path: &str) -> Result<()> {
        std::process::Command::new("xdg-open").arg(path).spawn()?;
        Ok(())
    }
}

fn autostart_path() -> Result<PathBuf> {
    let base = directories::BaseDirs::new()
        .ok_or_else(|| anyhow!("Could not resolve the user's base directories."))?;
    Ok(base
        .config_dir()
        .join("autostart")
        .join(format!("{}.desktop", crate::identity::APP_ID)))
}
