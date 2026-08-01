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

/// Helper to construct terminal emulator command line with app_id / WM_CLASS override flags.
pub fn build_terminal_command(term: &str, app_id: &str, exec_str: &str) -> String {
    let term_binary = term.split_whitespace().next().unwrap_or(term);
    let term_lower = term_binary.to_lowercase();
    let term_name = Path::new(&term_lower)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&term_lower);

    match term_name {
        "foot" | "footclient" => {
            format!("'{term}' -a '{app_id}' -e {exec_str}")
        }
        "alacritty" => {
            format!("'{term}' --class '{app_id}' -e {exec_str}")
        }
        "kitty" => {
            format!("'{term}' --class '{app_id}' {exec_str}")
        }
        "ghostty" => {
            format!("'{term}' --class='{app_id}' -e {exec_str}")
        }
        "cosmic-term" => {
            format!("'{term}' --app-id='{app_id}' -e {exec_str}")
        }
        "wezterm" => {
            format!("'{term}' start --class '{app_id}' -- {exec_str}")
        }
        _ => {
            format!("'{term}' -e {exec_str}")
        }
    }
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
            default_terminal: crate::utils::get_default_terminal_emulator(),
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

    /// Constructs the appropriate execution command vector for a target desktop app or app_id.
    pub fn build_command(
        &self,
        app_id: &str,
        desktop_path: Option<&Path>,
        is_terminal: bool,
        cleaned_exec: Option<&str>,
    ) -> Vec<String> {
        let path_str_opt = desktop_path.map(|p| p.to_string_lossy().to_string());

        let raw_cmd = if is_terminal {
            let exec_str = cleaned_exec.unwrap_or(app_id);
            if let Some(ref term) = self.default_terminal {
                let term_cmd = build_terminal_command(term, app_id, exec_str);
                if self.has_uwsm {
                    vec![
                        "sh".to_string(),
                        "-c".to_string(),
                        format!("exec uwsm app -- {term_cmd}"),
                    ]
                } else {
                    vec![
                        "sh".to_string(),
                        "-c".to_string(),
                        format!("exec {term_cmd}"),
                    ]
                }
            } else if self.has_uwsm {
                if let Some(ref path_str) = path_str_opt {
                    vec![
                        "sh".to_string(),
                        "-c".to_string(),
                        format!("exec uwsm app -- '{path_str}'"),
                    ]
                } else {
                    vec!["uwsm".to_string(), "app".to_string(), "--".to_string(), app_id.to_string()]
                }
            } else if self.has_gtk_launch {
                vec!["gtk-launch".to_string(), app_id.to_string()]
            } else if self.has_gio {
                if let Some(ref path_str) = path_str_opt {
                    vec!["gio".to_string(), "launch".to_string(), path_str.clone()]
                } else {
                    vec!["sh".to_string(), "-c".to_string(), format!("exec '{exec_str}'")]
                }
            } else {
                vec!["sh".to_string(), "-c".to_string(), format!("exec '{exec_str}'")]
            }
        } else if self.has_uwsm {
            if let Some(ref path_str) = path_str_opt {
                vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    format!("exec uwsm app -- '{path_str}'"),
                ]
            } else {
                vec!["uwsm".to_string(), "app".to_string(), "--".to_string(), app_id.to_string()]
            }
        } else if self.has_gtk_launch {
            vec!["gtk-launch".to_string(), app_id.to_string()]
        } else if self.has_gio {
            if let Some(ref path_str) = path_str_opt {
                vec!["gio".to_string(), "launch".to_string(), path_str.clone()]
            } else {
                vec!["sh".to_string(), "-c".to_string(), format!("exec '{app_id}'")]
            }
        } else if let Some(exec) = cleaned_exec {
            if !exec.is_empty() {
                vec!["sh".to_string(), "-c".to_string(), format!("exec {exec}")]
            } else if let Some(ref path_str) = path_str_opt {
                vec!["sh".to_string(), "-c".to_string(), format!("exec '{path_str}'")]
            } else {
                vec!["sh".to_string(), "-c".to_string(), format!("exec '{app_id}'")]
            }
        } else if let Some(ref path_str) = path_str_opt {
            vec!["sh".to_string(), "-c".to_string(), format!("exec '{path_str}'")]
        } else {
            vec!["sh".to_string(), "-c".to_string(), format!("exec '{app_id}'")]
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
