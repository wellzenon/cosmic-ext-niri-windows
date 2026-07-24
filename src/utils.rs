use std::path::{Path, PathBuf};
use freedesktop_desktop_entry::DesktopEntry;
use std::fs;

/// Standard application desktop entry directories.
fn get_application_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
    ];

    if let Some(d) = dirs::data_dir() {
        dirs.push(d.join("applications"));
        dirs.push(d.join("flatpak/exports/share/applications"));
    }

    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    dirs
}

pub fn resolve_icon_name_to_path(icon_name: &str) -> Option<String> {
    if icon_name.is_empty() {
        return None;
    }

    let path = PathBuf::from(icon_name);
    if path.is_absolute() && path.exists() {
        return Some(icon_name.to_string());
    }

    // Standard search directories for icons
    let mut search_dirs = Vec::new();
    if let Some(data_dir) = dirs::data_dir() {
        search_dirs.push(data_dir.join("icons"));
    }
    if let Some(home_dir) = dirs::home_dir() {
        search_dirs.push(home_dir.join(".icons"));
    }
    search_dirs.push(PathBuf::from("/usr/share/icons"));
    search_dirs.push(PathBuf::from("/usr/share/pixmaps"));
    search_dirs.push(PathBuf::from("/usr/local/share/icons"));
    search_dirs.push(PathBuf::from("/usr/local/share/pixmaps"));

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

    // 2. Check hicolor fallback theme first, as it's the standard fallback
    for dir in &search_dirs {
        if !dir.exists() || dir.ends_with("pixmaps") {
            continue;
        }
        let hicolor_dir = dir.join("hicolor");
        if hicolor_dir.exists() {
            if let Some(found_path) = search_in_dir_recursive(&hicolor_dir, &target_names, 0, 5) {
                return Some(found_path.to_string_lossy().into_owned());
            }
        }
    }

    // 3. Search other directories recursively (limiting depth to prevent long delays)
    for dir in &search_dirs {
        if !dir.exists() || dir.ends_with("pixmaps") {
            continue;
        }
        if let Some(found_path) = search_in_dir_recursive(dir, &target_names, 0, 5) {
            return Some(found_path.to_string_lossy().into_owned());
        }
    }

    None
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
                if current_depth == 0 && path.file_name().map_or(false, |n| n == "hicolor") {
                    continue;
                }
                subdirs.push(path);
            } else if path.is_file() {
                if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                    for target in target_names {
                        if filename == target {
                            return Some(path);
                        }
                    }
                }
            }
        }
        for subdir in subdirs {
            if let Some(found) = search_in_dir_recursive(&subdir, target_names, current_depth + 1, max_depth) {
                return Some(found);
            }
        }
    }
    None
}

/// Finds the fallback icon name or resolved path for a given app_id.
pub fn find_fallback_icon(app_id: &str) -> Option<String> {
    // 1. Delegate desktop entry discovery to find_desktop_file_path
    if let Some(desktop_path) = find_desktop_file_path(app_id) {
        if let Ok(entry) = DesktopEntry::from_path(&desktop_path, None::<&[&str]>) {
            if let Some(icon) = entry.icon() {
                if let Some(resolved_path) = resolve_icon_name_to_path(icon) {
                    return Some(resolved_path);
                }
                return Some(icon.to_string());
            }
        }
    }

    // 2. Fallback: resolve app_id directly as icon name
    resolve_icon_name_to_path(app_id)
}

/// Searches desktop files to retrieve the absolute path for a given app_id.
pub fn find_desktop_file_path(app_id: &str) -> Option<PathBuf> {
    let dirs = get_application_dirs();
    let app_id_lower = app_id.to_lowercase();
    let dot_app_id_desktop = format!(".{}.desktop", app_id_lower);

    let mut search_names = vec![
        format!("{}.desktop", app_id),
        format!("{}.desktop", app_id_lower),
    ];

    if app_id.contains('.') {
        if let Some(last_part) = app_id.split('.').last() {
            search_names.push(format!("{}.desktop", last_part));
            search_names.push(format!("{}.desktop", last_part.to_lowercase()));
        }
    }

    for dir in &dirs {
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
        if let Ok(entries) = fs::read_dir(dir) {
            for e in entries.flatten() {
                if let Ok(file_name) = e.file_name().into_string() {
                    let file_name_lower = file_name.to_lowercase();
                    if !file_name_lower.ends_with(".desktop") {
                        continue;
                    }

                    // 1. Check fuzzy filename match
                    if file_name_lower.starts_with(&app_id_lower)
                        || file_name_lower.ends_with(&dot_app_id_desktop)
                    {
                        return Some(e.path());
                    }

                    // 2. Check StartupWMClass match
                    let path = e.path();
                    if let Ok(entry) = DesktopEntry::from_path(&path, None::<&[&str]>) {
                        if let Some(group) = entry.groups.group("Desktop Entry") {
                            if let Some(wm_class) = group.0.get("StartupWMClass") {
                                if wm_class.0.eq_ignore_ascii_case(&app_id_lower) {
                                    return Some(path);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Reads the Name field from a desktop entry file.
pub fn get_desktop_entry_name(path: &Path) -> Option<String> {
    if let Ok(entry) = DesktopEntry::from_path(path, None::<&[&str]>) {
        if let Some(group) = entry.groups.group("Desktop Entry") {
            if let Some(name_tuple) = group.0.get("Name") {
                return Some(name_tuple.0.clone());
            }
        }
    }
    None
}
