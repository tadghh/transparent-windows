use anyhow::{Result, anyhow};
use std::path::PathBuf;

// TODO correct the hardcoded app values, use nix crate

pub fn process_name_from_pid(pid: u32) -> Result<String> {
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

fn autostart_path() -> Result<PathBuf> {
    let base = directories::BaseDirs::new()
        .ok_or_else(|| anyhow!("Could not resolve the user's base directories."))?;
    Ok(base.config_dir().join("autostart").join("winalpha.desktop"))
}

pub fn set_autostart(enabled: bool) -> Result<()> {
    let path = autostart_path()?;

    if enabled {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let exe = std::env::current_exe()?;
        let contents = format!(
            "[Desktop Entry]\nType=Application\nName=WinAlpha\nExec={}\nX-GNOME-Autostart-enabled=true\n",
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

pub fn get_autostart_state() -> bool {
    autostart_path().map(|path| path.exists()).unwrap_or(false)
}

pub fn open_path(path: &str) -> Result<()> {
    std::process::Command::new("xdg-open").arg(path).spawn()?;
    Ok(())
}
