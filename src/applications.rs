use freedesktop_desktop_entry::DesktopEntry;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static DESKTOP_PATH_CACHE: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
static APPLICATION_DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
static DESKTOP_FILES_BY_DIR: OnceLock<Mutex<HashMap<PathBuf, Vec<PathBuf>>>> = OnceLock::new();
static STARTUP_WM_CLASS_CACHE: OnceLock<Mutex<HashMap<PathBuf, Option<String>>>> = OnceLock::new();
const MAX_LOOKUP_CACHE_ENTRIES: usize = 2048;

fn application_dirs() -> &'static [PathBuf] {
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
            if let Some(data_dir) = dirs::data_dir() {
                dirs.push(data_dir.join("applications"));
                dirs.push(data_dir.join("flatpak/exports/share/applications"));
            }
            dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
            dirs
        })
        .as_slice()
}

/// Resolves the canonical desktop entry ID for a running application ID.
pub fn resolve_canonical_desktop_id(app_id: &str) -> String {
    // A browser web-app is a distinct desktop entry even when its
    // StartupWMClass matches the parent browser.
    if app_id.to_ascii_lowercase().contains("webapp") {
        return app_id.to_string();
    }
    find_desktop_file_path(app_id)
        .and_then(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| app_id.to_string())
}

/// Treats browser web-app aliases with the same generated ID as equivalent.
pub fn app_ids_equivalent(left: &str, right: &str) -> bool {
    left == right
        || left
            .split_once("webapp-")
            .zip(right.split_once("webapp-"))
            .is_some_and(|((_, left_id), (_, right_id))| left_id == right_id)
}

/// Searches desktop entry directories for an application ID.
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
    let app_id_lower = app_id.to_lowercase();
    let dot_app_id_desktop = format!(".{app_id_lower}.desktop");
    let mut search_names = vec![
        format!("{app_id}.desktop"),
        format!("{app_id_lower}.desktop"),
    ];
    if app_id.contains('.')
        && let Some(last_part) = app_id.split('.').next_back()
    {
        search_names.push(format!("{last_part}.desktop"));
        search_names.push(format!("{}.desktop", last_part.to_lowercase()));
    }

    for dir in application_dirs() {
        if !dir.exists() {
            continue;
        }
        for name in &search_names {
            let path = dir.join(name);
            if path.exists() {
                return Some(path);
            }
        }
        for path in desktop_files_in_dir(dir) {
            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let file_name_lower = file_name.to_lowercase();
            if !file_name_lower.ends_with(".desktop") {
                continue;
            }
            if file_name_lower.starts_with(&app_id_lower)
                || file_name_lower.ends_with(&dot_app_id_desktop)
            {
                return Some(path);
            }

            let cache = STARTUP_WM_CLASS_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
            let wm_class = if let Ok(mut cache) = cache.lock() {
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
            if wm_class
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(&app_id_lower))
            {
                return Some(path);
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

/// Reads the localized display name from a desktop entry.
pub fn get_desktop_entry_name(path: &Path) -> Option<String> {
    DesktopEntry::from_path(path, None::<&[&str]>)
        .ok()
        .and_then(|entry| entry.groups.group("Desktop Entry").cloned())
        .and_then(|group| group.0.get("Name").cloned())
        .map(|name| name.0)
}

#[cfg(test)]
mod tests {
    use super::app_ids_equivalent;

    #[test]
    fn webapp_aliases_with_the_same_id_are_equivalent() {
        assert!(app_ids_equivalent(
            "librewolf.webapp-1234",
            "org.mozilla.firefox.webapp-1234"
        ));
        assert!(!app_ids_equivalent(
            "librewolf.webapp-1234",
            "librewolf.webapp-5678"
        ));
    }
}
