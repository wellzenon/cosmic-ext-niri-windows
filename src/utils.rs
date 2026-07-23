use std::path::PathBuf;
use freedesktop_desktop_entry::DesktopEntry;
use std::fs;

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

    let target_names = [
        format!("{}.png", icon_name),
        format!("{}.svg", icon_name),
        format!("{}.xpm", icon_name),
        format!("{}.png", icon_name.to_lowercase()),
        format!("{}.svg", icon_name.to_lowercase()),
        format!("{}.xpm", icon_name.to_lowercase()),
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
        if !dir.exists() || dir.to_string_lossy().contains("pixmaps") {
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
        if !dir.exists() || dir.to_string_lossy().contains("pixmaps") {
            continue;
        }
        if let Some(found_path) = search_in_dir_recursive(dir, &target_names, 0, 5) {
            return Some(found_path.to_string_lossy().into_owned());
        }
    }

    None
}

fn search_in_dir_recursive(
    dir: &PathBuf,
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

pub fn find_fallback_icon(app_id: &str) -> Option<String> {
    // 1. First search for a desktop entry and retrieve the icon name from it
    let mut resolved_icon_name = None;

    let dirs = vec![
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
        dirs::data_dir().map(|d| d.join("applications")).unwrap_or_default(),
        dirs::data_dir().map(|d| d.join("flatpak/exports/share/applications")).unwrap_or_default(),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
    ];

    let mut search_names = vec![
        format!("{}.desktop", app_id),
        format!("{}.desktop", app_id.to_lowercase()),
    ];

    if app_id.contains('.') {
        if let Some(last_part) = app_id.split('.').last() {
            search_names.push(format!("{}.desktop", last_part));
            search_names.push(format!("{}.desktop", last_part.to_lowercase()));
        }
    }

    'outer: for dir in &dirs {
        if !dir.exists() {
            continue;
        }

        // Try exact match first
        for name in &search_names {
            let path = dir.join(name);
            if path.exists() {
                if let Ok(entry) = DesktopEntry::from_path(&path, None::<&[&str]>) {
                    if let Some(icon) = entry.icon() {
                        resolved_icon_name = Some(icon.to_string());
                        break 'outer;
                    }
                }
            }
        }
        
        // Then try fuzzy matching by filename
        if let Ok(entries) = fs::read_dir(dir) {
            let app_id_lower = app_id.to_lowercase();
            for e in entries.flatten() {
                if let Ok(file_name) = e.file_name().into_string() {
                    let file_name_lower = file_name.to_lowercase();
                    if (file_name_lower.starts_with(&app_id_lower)
                        || file_name_lower.ends_with(&format!(".{}.desktop", app_id_lower)))
                        && file_name_lower.ends_with(".desktop")
                    {
                        let path = e.path();
                        if let Ok(entry) = DesktopEntry::from_path(&path, None::<&[&str]>) {
                            if let Some(icon) = entry.icon() {
                                resolved_icon_name = Some(icon.to_string());
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }

        // Fallback: search by StartupWMClass inside the desktop files
        if let Ok(entries) = fs::read_dir(dir) {
            let app_id_lower = app_id.to_lowercase();
            for e in entries.flatten() {
                let path = e.path();
                if path.extension().map_or(false, |ext| ext == "desktop") {
                    if let Ok(entry) = DesktopEntry::from_path(&path, None::<&[&str]>) {
                        if let Some(group) = entry.groups.group("Desktop Entry") {
                            if let Some(wm_class) = group.0.get("StartupWMClass") {
                                if wm_class.0.to_lowercase() == app_id_lower {
                                    if let Some(icon) = entry.icon() {
                                        resolved_icon_name = Some(icon.to_string());
                                        break 'outer;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. If we found an icon name, try to resolve it to an absolute path first
    if let Some(icon_name) = &resolved_icon_name {
        if let Some(path) = resolve_icon_name_to_path(icon_name) {
            return Some(path);
        }
        return Some(icon_name.clone());
    }

    // 3. If we didn't find any desktop entry, try to resolve the app_id itself to an absolute path
    if let Some(path) = resolve_icon_name_to_path(app_id) {
        return Some(path);
    }

    // Otherwise, return None so the caller falls back to app_id as the icon name
    None
}

/// Searches desktop files to retrieve the absolute path for a given app_id.
pub fn find_desktop_file_path(app_id: &str) -> Option<std::path::PathBuf> {
    let dirs = vec![
        std::path::PathBuf::from("/usr/share/applications"),
        std::path::PathBuf::from("/usr/local/share/applications"),
        dirs::data_dir().map(|d| d.join("applications")).unwrap_or_default(),
        dirs::data_dir().map(|d| d.join("flatpak/exports/share/applications")).unwrap_or_default(),
        std::path::PathBuf::from("/var/lib/flatpak/exports/share/applications"),
    ];

    let mut search_names = vec![
        format!("{}.desktop", app_id),
        format!("{}.desktop", app_id.to_lowercase()),
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

        // Try fuzzy match
        if let Ok(entries) = std::fs::read_dir(dir) {
            let app_id_lower = app_id.to_lowercase();
            for e in entries.flatten() {
                if let Ok(file_name) = e.file_name().into_string() {
                    let file_name_lower = file_name.to_lowercase();
                    if (file_name_lower.starts_with(&app_id_lower)
                        || file_name_lower.ends_with(&format!(".{}.desktop", app_id_lower)))
                        && file_name_lower.ends_with(".desktop")
                    {
                        return Some(e.path());
                    }
                }
            }
        }

        // Try StartupWMClass match
        if let Ok(entries) = std::fs::read_dir(dir) {
            let app_id_lower = app_id.to_lowercase();
            for e in entries.flatten() {
                let path = e.path();
                if path.extension().map_or(false, |ext| ext == "desktop") {
                    if let Ok(entry) = DesktopEntry::from_path(&path, None::<&[&str]>) {
                        if let Some(group) = entry.groups.group("Desktop Entry") {
                            if let Some(wm_class) = group.0.get("StartupWMClass") {
                                if wm_class.0.to_lowercase() == app_id_lower {
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
pub fn get_desktop_entry_name(path: &std::path::Path) -> Option<String> {
    if let Ok(entry) = DesktopEntry::from_path(path, None::<&[&str]>) {
        if let Some(group) = entry.groups.group("Desktop Entry") {
            if let Some(name_tuple) = group.0.get("Name") {
                return Some(name_tuple.0.clone());
            }
        }
    }
    None
}
