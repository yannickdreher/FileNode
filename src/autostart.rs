use anyhow::Result;
use std::path::Path;

pub fn set(enabled: bool, profile: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")?;
        if enabled {
            let executable = std::env::current_exe()?;
            let profile = std::path::absolute(profile)?;
            key.set_value(
                "NoritecDrive",
                &format!(
                    "\"{}\" --background --config \"{}\"",
                    executable.display(),
                    profile.display()
                ),
            )?;
        } else {
            match key.delete_value("NoritecDrive") {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        let directory = directories::BaseDirs::new()
            .ok_or_else(|| anyhow::anyhow!("User configuration directory unavailable"))?
            .config_dir()
            .join("autostart");
        let target = directory.join("noritec-drive.desktop");
        if enabled {
            let executable = std::env::current_exe()?;
            let profile = std::path::absolute(profile)?;
            let body = format!("[Desktop Entry]\nType=Application\nName=Noritec Drive\nExec={} --background --config {}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n", quote_exec(&executable)?, quote_exec(&profile)?);
            std::fs::create_dir_all(directory)?;
            std::fs::write(target, body)?;
        } else {
            match std::fs::remove_file(target) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn quote_exec(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Autostart path is not UTF-8"))?;
    anyhow::ensure!(!value.contains(['\n', '\r']), "Invalid autostart path");
    let value = value
        .replace('\\', "\\\\\\\\")
        .replace('"', "\\\\\"")
        .replace('`', "\\\\`")
        .replace('$', "\\\\$")
        .replace('%', "%%");
    Ok(format!("\"{value}\""))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn desktop_exec_paths_are_quoted_and_field_codes_are_literal() {
        assert_eq!(
            quote_exec(Path::new("/home/a b/100%/drive")).unwrap(),
            "\"/home/a b/100%%/drive\""
        );
        assert!(quote_exec(Path::new("/tmp/a\nInjected=true")).is_err());
    }
}
