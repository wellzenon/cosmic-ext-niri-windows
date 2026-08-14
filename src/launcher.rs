use std::fs;
use std::path::Path;

/// Helper to check if running inside a Flatpak container.
pub fn is_flatpak() -> bool {
    Path::new("/.flatpak-info").exists() || std::env::var_os("FLATPAK_ID").is_some()
}

/// Helper to check if an executable binary exists in system PATH or on the host.
pub fn binary_exists(name: &str) -> bool {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let full_path = dir.join(name);
            if full_path.is_file() {
                return true;
            }
        }
    }
    if is_flatpak()
        && let Ok(output) = std::process::Command::new("flatpak-spawn")
            .args(["--host", "which", name])
            .output()
    {
        return output.status.success();
    }
    false
}

/// Cached system launcher capabilities for application dispatching.
#[derive(Debug, Clone)]
pub struct SystemLauncher {
    pub is_flatpak: bool,
    pub has_uwsm: bool,
    pub has_gtk_launch: bool,
    pub has_gio: bool,
    pub default_terminal: Option<String>,
}

impl SystemLauncher {
    /// Detects system capabilities once at application startup and caches the result.
    pub fn detect() -> Self {
        let is_flatpak_env = is_flatpak();
        let launcher = Self {
            is_flatpak: is_flatpak_env,
            has_uwsm: binary_exists("uwsm"),
            has_gtk_launch: binary_exists("gtk-launch"),
            has_gio: binary_exists("gio"),
            default_terminal: default_terminal_emulator(),
        };
        log::info!(
            "SystemLauncher detected: flatpak={}, uwsm={}, gtk-launch={}, gio={}, default_terminal={:?}",
            launcher.is_flatpak,
            launcher.has_uwsm,
            launcher.has_gtk_launch,
            launcher.has_gio,
            launcher.default_terminal
        );
        launcher
    }

    /// Builds a direct argv vector. No user-controlled value is passed through a shell.
    pub fn build_command_with_args(
        &self,
        app_id: &str,
        desktop_path: Option<&Path>,
        is_terminal: bool,
        exec_args: Option<&[String]>,
    ) -> Vec<String> {
        let path_str_opt = desktop_path.map(|p| p.to_string_lossy().to_string());
        let fallback_args = || {
            exec_args
                .filter(|args| !args.is_empty())
                .map_or_else(|| vec![app_id.to_string()], ToOwned::to_owned)
        };

        let raw_cmd = if is_terminal {
            if let Some(ref term) = self.default_terminal {
                let mut term_cmd = term
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                if term_cmd.is_empty() {
                    return fallback_args();
                }
                let term_name = Path::new(&term_cmd[0])
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&term_cmd[0])
                    .to_ascii_lowercase();
                match term_name.as_str() {
                    "foot" | "footclient" => {
                        term_cmd.extend(["-a".into(), app_id.into(), "-e".into()])
                    }
                    "alacritty" => term_cmd.extend(["--class".into(), app_id.into(), "-e".into()]),
                    "kitty" => term_cmd.extend(["--class".into(), app_id.into()]),
                    "ghostty" => term_cmd.extend([format!("--class={app_id}"), "-e".into()]),
                    "cosmic-term" => term_cmd.extend([format!("--app-id={app_id}"), "-e".into()]),
                    "wezterm" => term_cmd.extend([
                        "start".into(),
                        "--class".into(),
                        app_id.into(),
                        "--".into(),
                    ]),
                    _ => term_cmd.push("-e".into()),
                }
                term_cmd.extend(fallback_args());
                if self.has_uwsm {
                    let mut command = vec!["uwsm".to_string(), "app".to_string(), "--".to_string()];
                    command.extend(term_cmd);
                    command
                } else {
                    term_cmd
                }
            } else if self.has_uwsm {
                if let Some(ref path_str) = path_str_opt {
                    vec!["uwsm".into(), "app".into(), "--".into(), path_str.clone()]
                } else {
                    let mut command = vec!["uwsm".into(), "app".into(), "--".into()];
                    command.extend(fallback_args());
                    command
                }
            } else if self.has_gtk_launch {
                vec!["gtk-launch".into(), app_id.to_string()]
            } else if self.has_gio {
                if let Some(ref path_str) = path_str_opt {
                    vec!["gio".into(), "launch".into(), path_str.clone()]
                } else {
                    fallback_args()
                }
            } else {
                fallback_args()
            }
        } else if self.has_uwsm {
            if let Some(ref path_str) = path_str_opt {
                vec!["uwsm".into(), "app".into(), "--".into(), path_str.clone()]
            } else {
                let mut command = vec!["uwsm".into(), "app".into(), "--".into()];
                command.extend(fallback_args());
                command
            }
        } else if self.has_gtk_launch {
            vec!["gtk-launch".into(), app_id.to_string()]
        } else if self.has_gio {
            if let Some(ref path_str) = path_str_opt {
                vec!["gio".into(), "launch".into(), path_str.clone()]
            } else {
                fallback_args()
            }
        } else if exec_args.is_some_and(|args| !args.is_empty()) {
            fallback_args()
        } else if let Some(ref path_str) = path_str_opt {
            vec![path_str.clone()]
        } else {
            fallback_args()
        };

        let final_cmd = if self.is_flatpak {
            let mut flatpak_cmd = vec!["flatpak-spawn".to_string(), "--host".to_string()];
            flatpak_cmd.extend(raw_cmd);
            flatpak_cmd
        } else {
            raw_cmd
        };

        log::debug!("Launch command for '{}': {:?}", app_id, final_cmd);
        final_cmd
    }
}

/// Resolves the user's configured terminal command without hardcoding a
/// specific emulator.
fn default_terminal_emulator() -> Option<String> {
    if let Ok(terminal) = std::env::var("TERMINAL") {
        let terminal = terminal.trim();
        if !terminal.is_empty() {
            return Some(terminal.to_string());
        }
    }
    if let Some(config_dir) = dirs::config_dir() {
        for file in ["xdg-terminals.list", "xdg-default-terminal.list"] {
            let path = config_dir.join(file);
            let Ok(content) = fs::read_to_string(path) else {
                continue;
            };
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let desktop_id = line.strip_suffix(".desktop").unwrap_or(line);
                let binary = desktop_id.split('.').next_back().unwrap_or(desktop_id);
                if !binary.is_empty() {
                    return Some(binary.to_string());
                }
            }
        }
    }
    for (program, args) in [
        (
            "gsettings",
            &[
                "get",
                "org.gnome.desktop.default-applications.terminal",
                "exec",
            ][..],
        ),
        (
            "dconf",
            &[
                "read",
                "/org/gnome/desktop/default-applications/terminal/exec",
            ][..],
        ),
    ] {
        let Ok(output) = std::process::Command::new(program).args(args).output() else {
            continue;
        };
        let terminal = String::from_utf8_lossy(&output.stdout)
            .trim()
            .trim_matches('\'')
            .trim_matches('"')
            .to_string();
        if !terminal.is_empty() {
            return Some(terminal);
        }
    }
    None
}
