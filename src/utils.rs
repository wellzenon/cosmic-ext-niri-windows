use freedesktop_desktop_entry::DesktopEntry;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static CURRENT_ICON_THEME: OnceLock<Option<String>> = OnceLock::new();
static DESKTOP_PATH_CACHE: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
static ICON_PATH_CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
static APPLICATION_DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
static DESKTOP_FILES_BY_DIR: OnceLock<Mutex<HashMap<PathBuf, Vec<PathBuf>>>> = OnceLock::new();
static STARTUP_WM_CLASS_CACHE: OnceLock<Mutex<HashMap<PathBuf, Option<String>>>> = OnceLock::new();
const MAX_LOOKUP_CACHE_ENTRIES: usize = 2048;

/// Standard application desktop entry directories.
fn get_application_dirs() -> &'static [PathBuf] {
    APPLICATION_DIRS
        .get_or_init(|| {
            let mut dirs = vec![
                PathBuf::from("/usr/share/applications"),
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/run/host/usr/share/applications"),
                PathBuf::from("/run/host/share/applications"),
            ];

            if let Some(home_dir) = dirs::home_dir() {
                dirs.push(home_dir.join(".local/share/applications"));
                dirs.push(home_dir.join(".local/share/flatpak/exports/share/applications"));
            }

            if let Some(d) = dirs::data_dir() {
                dirs.push(d.join("applications"));
                dirs.push(d.join("flatpak/exports/share/applications"));
            }

            dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
            dirs
        })
        .as_slice()
}

pub fn resolve_icon_name_to_path(icon_name: &str) -> Option<String> {
    let cache = ICON_PATH_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache.lock().ok()?.get(icon_name).cloned() {
        return cached;
    }

    let result = resolve_icon_name_to_path_uncached(icon_name);
    if let Ok(mut entries) = cache.lock() {
        if entries.len() >= MAX_LOOKUP_CACHE_ENTRIES {
            entries.clear();
        }
        entries.insert(icon_name.to_owned(), result.clone());
    }
    result
}

fn resolve_icon_name_to_path_uncached(icon_name: &str) -> Option<String> {
    if icon_name.is_empty() {
        return None;
    }

    let path = PathBuf::from(icon_name);
    if path.is_absolute() && path.exists() {
        return Some(icon_name.to_string());
    }

    let mut search_dirs = Vec::new();
    if crate::launcher::is_flatpak() {
        search_dirs.push(PathBuf::from("/run/host/usr/share/icons"));
        search_dirs.push(PathBuf::from("/run/host/usr/share/pixmaps"));
        search_dirs.push(PathBuf::from("/run/host/share/icons"));
        if let Some(home_dir) = dirs::home_dir() {
            search_dirs.push(home_dir.join(".local/share/icons"));
            search_dirs.push(home_dir.join(".icons"));
        }
    } else {
        if let Some(home_dir) = dirs::home_dir() {
            search_dirs.push(home_dir.join(".local/share/icons"));
            search_dirs.push(home_dir.join(".icons"));
        }
        if let Some(data_dir) = dirs::data_dir() {
            search_dirs.push(data_dir.join("icons"));
        }
        search_dirs.push(PathBuf::from("/usr/share/icons"));
        search_dirs.push(PathBuf::from("/usr/share/pixmaps"));
        search_dirs.push(PathBuf::from("/usr/local/share/icons"));
        search_dirs.push(PathBuf::from("/usr/local/share/pixmaps"));
        search_dirs.push(PathBuf::from("/run/host/usr/share/icons"));
        search_dirs.push(PathBuf::from("/run/host/usr/share/pixmaps"));
        search_dirs.push(PathBuf::from("/run/host/share/icons"));
    }

    let icon_lower = icon_name.to_lowercase();
    let target_names = [
        format!("{}.png", icon_name),
        format!("{}.svg", icon_name),
        format!("{}.xpm", icon_name),
        format!("{}.png", icon_lower),
        format!("{}.svg", icon_lower),
        format!("{}.xpm", icon_lower),
    ];

    // 1. Check top-level (especially useful for pixmaps)
    for dir in &search_dirs {
        if !dir.exists() {
            continue;
        }
        for target in &target_names {
            let file_path = dir.join(target);
            if file_path.exists() {
                return Some(file_path.to_string_lossy().into_owned());
            }
        }
    }

    // 2. Check active user icon theme first if set
    if let Some(theme) = get_current_icon_theme() {
        for dir in &search_dirs {
            if !dir.exists() || dir.ends_with("pixmaps") {
                continue;
            }
            let theme_dir = dir.join(&theme);
            if theme_dir.exists()
                && let Some(found_path) = find_common_icon_path(&theme_dir, &target_names)
            {
                return Some(found_path.to_string_lossy().into_owned());
            }
        }
    }

    // 3. Check hicolor fallback theme first, as it's the standard fallback
    for dir in &search_dirs {
        if !dir.exists() || dir.ends_with("pixmaps") {
            continue;
        }
        let hicolor_dir = dir.join("hicolor");
        if hicolor_dir.exists()
            && let Some(found_path) = find_common_icon_path(&hicolor_dir, &target_names)
        {
            return Some(found_path.to_string_lossy().into_owned());
        }
    }

    // Do not recursively scan every installed theme. Apart from being expensive,
    // that would select an arbitrary icon from an inactive theme. The active theme
    // and hicolor fallback above are the freedesktop lookup contract.
    None
}

fn find_common_icon_path(dir: &Path, target_names: &[String]) -> Option<PathBuf> {
    const COMMON_ICON_DIRS: &[&str] = &[
        "apps/scalable",
        "scalable/apps",
        "apps/128x128",
        "apps/64x64",
        "apps/48x48",
        "apps/32x32",
        "apps/24x24",
        "apps/22x22",
        "apps/16x16",
    ];

    for relative in COMMON_ICON_DIRS {
        let candidate_dir = dir.join(relative);
        for target in target_names {
            let candidate = candidate_dir.join(target);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    search_in_dir_recursive(dir, target_names, 0, 5)
}

/// Retrieves the user's currently configured GTK icon theme name.
pub fn get_current_icon_theme() -> Option<String> {
    CURRENT_ICON_THEME
        .get_or_init(|| {
            if let Ok(output) = std::process::Command::new("gsettings")
                .args(["get", "org.gnome.desktop.interface", "icon-theme"])
                .output()
            {
                let theme = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .trim_matches('\'')
                    .trim_matches('"')
                    .to_string();
                if !theme.is_empty() {
                    return Some(theme);
                }
            }
            None
        })
        .clone()
}

fn search_in_dir_recursive(
    dir: &Path,
    target_names: &[String],
    current_depth: usize,
    max_depth: usize,
) -> Option<PathBuf> {
    if current_depth > max_depth {
        return None;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Avoid recursing into hicolor again if we've already done that specifically
                if current_depth == 0 && path.file_name().is_some_and(|n| n == "hicolor") {
                    continue;
                }
                subdirs.push(path);
            } else if path.is_file()
                && let Some(filename) = path.file_name().and_then(|f| f.to_str())
            {
                for target in target_names {
                    if filename == target {
                        return Some(path);
                    }
                }
            }
        }
        for subdir in subdirs {
            if let Some(found) =
                search_in_dir_recursive(&subdir, target_names, current_depth + 1, max_depth)
            {
                return Some(found);
            }
        }
    }
    None
}

/// Finds the fallback icon name or resolved path for a given app_id.
pub fn find_fallback_icon(app_id: &str) -> Option<String> {
    // 1. Delegate desktop entry discovery to find_desktop_file_path
    if let Some(desktop_path) = find_desktop_file_path(app_id)
        && let Ok(entry) = DesktopEntry::from_path(&desktop_path, None::<&[&str]>)
        && let Some(icon) = entry.icon()
    {
        return Some(icon.to_string());
    }

    // 2. Fallback: resolve app_id directly as icon name
    resolve_icon_name_to_path(app_id)
}
/// Resolves the canonical desktop entry ID (e.g. "org.neovim.nvim") for a given running app_id (e.g. "nvim").
pub fn resolve_canonical_desktop_id(app_id: &str) -> String {
    // Browser web-app IDs identify a distinct desktop entry. Resolving them
    // through StartupWMClass would collapse them into the parent browser and
    // cause session restore to launch a normal browser window instead.
    if app_id.to_ascii_lowercase().contains("webapp") {
        return app_id.to_string();
    }
    if let Some(desktop_path) = find_desktop_file_path(app_id)
        && let Some(stem) = desktop_path.file_stem().and_then(|s| s.to_str())
    {
        return stem.to_string();
    }
    app_id.to_string()
}

/// Searches desktop files to retrieve the absolute path for a given app_id.
pub fn find_desktop_file_path(app_id: &str) -> Option<PathBuf> {
    let cache = DESKTOP_PATH_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache.lock().ok()?.get(app_id).cloned() {
        return cached;
    }

    let result = find_desktop_file_path_uncached(app_id);
    if let Ok(mut entries) = cache.lock() {
        if entries.len() >= MAX_LOOKUP_CACHE_ENTRIES {
            entries.clear();
        }
        entries.insert(app_id.to_owned(), result.clone());
    }
    result
}

fn find_desktop_file_path_uncached(app_id: &str) -> Option<PathBuf> {
    let dirs = get_application_dirs();
    let app_id_lower = app_id.to_lowercase();
    let dot_app_id_desktop = format!(".{}.desktop", app_id_lower);

    let mut search_names = vec![
        format!("{}.desktop", app_id),
        format!("{}.desktop", app_id_lower),
    ];

    if app_id.contains('.')
        && let Some(last_part) = app_id.split('.').next_back()
    {
        search_names.push(format!("{}.desktop", last_part));
        search_names.push(format!("{}.desktop", last_part.to_lowercase()));
    }

    for dir in dirs {
        if !dir.exists() {
            continue;
        }

        // Try exact match first
        for name in &search_names {
            let path = dir.join(name);
            if path.exists() {
                return Some(path);
            }
        }

        // Single-pass directory scan for fuzzy filename match & StartupWMClass match
        for path in desktop_files_in_dir(dir) {
            if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
                let file_name_lower = file_name.to_lowercase();
                if !file_name_lower.ends_with(".desktop") {
                    continue;
                }

                // 1. Check fuzzy filename match
                if file_name_lower.starts_with(&app_id_lower)
                    || file_name_lower.ends_with(&dot_app_id_desktop)
                {
                    return Some(path);
                }

                // 2. Check StartupWMClass match
                let startup_wm_class =
                    STARTUP_WM_CLASS_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
                let wm_class = if let Ok(mut cache) = startup_wm_class.lock() {
                    cache
                        .entry(path.clone())
                        .or_insert_with(|| {
                            DesktopEntry::from_path(&path, None::<&[&str]>)
                                .ok()
                                .and_then(|entry| entry.groups.group("Desktop Entry").cloned())
                                .and_then(|group| group.0.get("StartupWMClass").cloned())
                                .map(|value| value.0)
                        })
                        .clone()
                } else {
                    None
                };
                let is_wm_class_match = wm_class
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case(&app_id_lower));

                if is_wm_class_match {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn desktop_files_in_dir(dir: &Path) -> Vec<PathBuf> {
    let cache = DESKTOP_FILES_BY_DIR.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(entries) = cache.lock()
        && let Some(paths) = entries.get(dir)
    {
        return paths.clone();
    }

    let paths = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "desktop"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Ok(mut entries) = cache.lock() {
        entries.insert(dir.to_owned(), paths.clone());
    }
    paths
}

/// Reads the Name field from a desktop entry file.
pub fn get_desktop_entry_name(path: &Path) -> Option<String> {
    DesktopEntry::from_path(path, None::<&[&str]>)
        .ok()
        .and_then(|entry| entry.groups.group("Desktop Entry").cloned())
        .and_then(|group| group.0.get("Name").cloned())
        .map(|name_tuple| name_tuple.0)
}

#[derive(Debug, Clone)]
pub struct ColumnPreset {
    pub label: String,
    pub change_spec: String,
}

/// Dynamically parses preset-column-widths from ~/.config/niri/config.kdl
pub fn load_niri_column_presets() -> Vec<ColumnPreset> {
    let mut presets = Vec::new();

    let mut possible_paths = Vec::new();
    if let Some(home_dir) = dirs::home_dir() {
        possible_paths.push(home_dir.join(".config").join("niri").join("config.kdl"));
    }
    if let Some(config_dir) = dirs::config_dir() {
        possible_paths.push(config_dir.join("niri").join("config.kdl"));
    }

    for niri_config in possible_paths {
        if let Ok(content) = fs::read_to_string(&niri_config) {
            let mut in_preset_block = false;
            for line in content.lines() {
                // Strip KDL comments
                let line_clean = if let Some(idx) = line.find("//") {
                    &line[..idx]
                } else {
                    line
                };
                let trimmed = line_clean.trim();
                if trimmed.is_empty() {
                    continue;
                }

                if trimmed.contains("preset-column-widths") {
                    in_preset_block = true;
                    continue;
                }
                if in_preset_block {
                    if trimmed.contains('}') {
                        break;
                    }
                    // Clean trailing semicolons or braces
                    let clean_line = trimmed.trim_matches(|c| c == ';' || c == '{' || c == '}');
                    let mut parts = clean_line.split_whitespace();
                    if let (Some(kind), Some(val_str)) = (parts.next(), parts.next()) {
                        if kind == "proportion" {
                            let raw_val = val_str.trim_matches(';');
                            if let Ok(val) = raw_val.parse::<f64>() {
                                let pct = (val * 100.0).round() as u32;
                                presets.push(ColumnPreset {
                                    label: format!("{pct}%"),
                                    change_spec: format!("{pct}%"),
                                });
                            }
                        } else if kind == "fixed" {
                            let raw_val = val_str.trim_matches(';');
                            if let Ok(val) = raw_val.parse::<i32>() {
                                presets.push(ColumnPreset {
                                    label: format!("{val}px"),
                                    change_spec: format!("{val}"),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    let raw_presets = if presets.is_empty() {
        // Fallback default presets if config.kdl does not specify preset-column-widths
        vec![
            ColumnPreset {
                label: "33%".to_string(),
                change_spec: "33%".to_string(),
            },
            ColumnPreset {
                label: "50%".to_string(),
                change_spec: "50%".to_string(),
            },
            ColumnPreset {
                label: "66%".to_string(),
                change_spec: "66%".to_string(),
            },
        ]
    } else {
        presets
    };

    let mut unique_presets = Vec::new();
    for p in raw_presets {
        if !unique_presets
            .iter()
            .any(|existing: &ColumnPreset| existing.label == p.label)
        {
            unique_presets.push(p);
        }
    }
    log::debug!(
        "Loaded {} column presets from niri config",
        unique_presets.len()
    );
    unique_presets
}

/// Dynamically resolves the user's configured default terminal emulator command/binary
/// without hardcoding specific terminal names.
pub fn get_default_terminal_emulator() -> Option<String> {
    // 1. $TERMINAL environment variable
    if let Ok(term) = std::env::var("TERMINAL") {
        let trimmed = term.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    // 2. ~/.config/xdg-terminals.list or ~/.config/xdg-default-terminal.list
    if let Some(config_dir) = dirs::config_dir() {
        for file in &["xdg-terminals.list", "xdg-default-terminal.list"] {
            let path = config_dir.join(file);
            if let Ok(content) = fs::read_to_string(path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    let clean = trimmed.strip_suffix(".desktop").unwrap_or(trimmed);
                    let binary = clean.split('.').next_back().unwrap_or(clean);
                    if !binary.is_empty() {
                        return Some(binary.to_string());
                    }
                }
            }
        }
    }

    // 3. gsettings org.gnome.desktop.default-applications.terminal exec
    if let Ok(output) = std::process::Command::new("gsettings")
        .args([
            "get",
            "org.gnome.desktop.default-applications.terminal",
            "exec",
        ])
        .output()
    {
        let s = String::from_utf8_lossy(&output.stdout)
            .trim()
            .trim_matches('\'')
            .trim_matches('"')
            .to_string();
        if !s.is_empty() {
            return Some(s);
        }
    }

    // 4. dconf /org/gnome/desktop/default-applications/terminal/exec
    if let Ok(output) = std::process::Command::new("dconf")
        .args([
            "read",
            "/org/gnome/desktop/default-applications/terminal/exec",
        ])
        .output()
    {
        let s = String::from_utf8_lossy(&output.stdout)
            .trim()
            .trim_matches('\'')
            .trim_matches('"')
            .to_string();
        if !s.is_empty() {
            return Some(s);
        }
    }

    None
}

/// Formats an RFC 3339 timestamp relative to a supplied instant.
///
/// Supplying `now` keeps this function deterministic and lets callers choose
/// when the displayed age should be refreshed.
pub fn format_relative_time(value: &str, now: jiff::Timestamp) -> String {
    let Ok(timestamp) = value.parse::<jiff::Timestamp>() else {
        return value.to_string();
    };
    let seconds = now.as_second().saturating_sub(timestamp.as_second()).max(0) as u64;
    match seconds {
        0..60 => "just now".to_string(),
        60..3_600 => format!("{} min ago", seconds / 60),
        3_600..86_400 => format!("{}h ago", seconds / 3_600),
        86_400..2_592_000 => {
            let days = seconds / 86_400;
            format!("{days} day{} ago", if days == 1 { "" } else { "s" })
        }
        2_592_000..31_536_000 => {
            let months = seconds / 2_592_000;
            format!("{months} month{} ago", if months == 1 { "" } else { "s" })
        }
        _ => {
            let years = seconds / 31_536_000;
            format!("{years} year{} ago", if years == 1 { "" } else { "s" })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format_relative_time;

    #[test]
    fn relative_time_uses_compact_units() {
        let now: jiff::Timestamp = "2026-08-11T12:00:00Z".parse().unwrap();
        assert_eq!(
            format_relative_time("2026-08-11T11:59:00Z", now),
            "1 min ago"
        );
        assert_eq!(format_relative_time("2026-08-11T10:00:00Z", now), "2h ago");
        assert_eq!(
            format_relative_time("2026-08-09T12:00:00Z", now),
            "2 days ago"
        );
    }
}
