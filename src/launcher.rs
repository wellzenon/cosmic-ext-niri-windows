use freedesktop_desktop_entry::DesktopEntry;
use std::fs;
use std::path::Path;

/// Checks whether the applet itself is running inside a Flatpak container.
pub fn is_flatpak() -> bool {
    Path::new("/.flatpak-info").exists() || std::env::var_os("FLATPAK_ID").is_some()
}

fn binary_exists(name: &str) -> bool {
    if std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join(name).is_file())
    }) {
        return true;
    }

    is_flatpak()
        && std::process::Command::new("flatpak-spawn")
            .args(["--host", "which", name])
            .output()
            .is_ok_and(|output| output.status.success())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopLaunch {
    pub app_id: String,
    pub exec: String,
    pub terminal: bool,
}

pub fn resolve_desktop_launch(path: &Path) -> Result<DesktopLaunch, String> {
    let entry = DesktopEntry::from_path(path, None::<&[&str]>)
        .map_err(|error| format!("could not read desktop entry {}: {error}", path.display()))?;
    let exec = entry
        .exec()
        .ok_or_else(|| format!("desktop entry {} has no Exec field", path.display()))?;

    Ok(DesktopLaunch {
        app_id: entry.appid.to_string(),
        exec: exec.to_owned(),
        terminal: entry.terminal(),
    })
}

/// Builds an argv command that lets UWSM create the application scope while
/// preserving the quoting and arguments from the desktop entry's Exec field.
pub fn build_launch_command(
    requested_app_id: &str,
    desktop_path: Option<&Path>,
) -> Result<Vec<String>, String> {
    let launch = match desktop_path {
        Some(path) => resolve_desktop_launch(path)?,
        None => DesktopLaunch {
            app_id: requested_app_id.to_owned(),
            exec: requested_app_id.to_owned(),
            terminal: false,
        },
    };
    let application = prepare_exec_args(&launch.exec, &launch.app_id, launch.terminal)?;

    let mut command = Vec::new();
    if is_flatpak() {
        command.extend(["flatpak-spawn".into(), "--host".into()]);
    }
    if binary_exists("uwsm") {
        command.extend(["uwsm".into(), "app".into(), "--".into()]);
    }
    command.extend(application);

    log::info!(
        "Launching '{}' through resolved desktop argv: {:?}",
        requested_app_id,
        command
    );
    Ok(command)
}

fn prepare_exec_args(exec: &str, app_id: &str, terminal: bool) -> Result<Vec<String>, String> {
    let application = shlex::split(exec)
        .filter(|args| !args.is_empty())
        .ok_or_else(|| format!("invalid Exec field for desktop entry {app_id}: {exec:?}"))?
        .into_iter()
        // Match libcosmic's current behavior: standalone desktop field codes
        // are ignored when no files or URLs were supplied by the caller.
        .filter(|arg| !arg.starts_with('%'))
        .collect::<Vec<_>>();

    if !terminal {
        return Ok(application);
    }

    let terminal = default_terminal_emulator()
        .ok_or_else(|| format!("no terminal configured for desktop entry {app_id}"))?;
    prepare_terminal_args(application, app_id, &terminal)
}

fn prepare_terminal_args(
    application: Vec<String>,
    app_id: &str,
    terminal: &str,
) -> Result<Vec<String>, String> {
    let mut command = shlex::split(terminal)
        .filter(|args| !args.is_empty())
        .ok_or_else(|| format!("invalid terminal command {terminal:?}"))?;
    let terminal_name = Path::new(&command[0])
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&command[0])
        .to_ascii_lowercase();

    match terminal_name.as_str() {
        "foot" | "footclient" => command.extend(["-a".into(), app_id.into(), "-e".into()]),
        "alacritty" => command.extend(["--class".into(), app_id.into(), "-e".into()]),
        "kitty" => command.extend(["--class".into(), app_id.into()]),
        "ghostty" => command.extend([format!("--class={app_id}"), "-e".into()]),
        "cosmic-term" => command.extend([format!("--app-id={app_id}"), "-e".into()]),
        "wezterm" => command.extend(["start".into(), "--class".into(), app_id.into(), "--".into()]),
        _ => command.push("-e".into()),
    }
    command.extend(application);
    Ok(command)
}

fn default_terminal_emulator() -> Option<String> {
    if let Ok(terminal) = std::env::var("TERMINAL") {
        let terminal = terminal.trim();
        if !terminal.is_empty() {
            return Some(terminal.to_owned());
        }
    }

    let config_dir = dirs::config_dir()?;
    for file in ["xdg-terminals.list", "xdg-default-terminal.list"] {
        let Ok(content) = fs::read_to_string(config_dir.join(file)) else {
            continue;
        };
        if let Some(desktop_id) = content
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
        {
            return Some(
                desktop_id
                    .strip_suffix(".desktop")
                    .unwrap_or(desktop_id)
                    .to_owned(),
            );
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{prepare_exec_args, prepare_terminal_args};

    #[test]
    fn terminal_exec_uses_configured_terminal_and_removes_field_codes() {
        let application = prepare_exec_args("nvim %F", "org.neovim.nvim", false).unwrap();
        let command = prepare_terminal_args(application, "org.neovim.nvim", "foot").unwrap();

        assert_eq!(command, ["foot", "-a", "org.neovim.nvim", "-e", "nvim"]);
    }

    #[test]
    fn quoted_shell_command_remains_one_argument() {
        let command = prepare_exec_args(
            "sh -c \"/usr/bin/losange -o '%u'\"",
            "xyz.timtimtim.Losange",
            false,
        )
        .unwrap();

        assert_eq!(command, ["sh", "-c", "/usr/bin/losange -o '%u'"]);
    }
}
