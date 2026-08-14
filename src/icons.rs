use freedesktop_desktop_entry::DesktopEntry;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

type IconPathCache = HashMap<(String, String), Option<String>>;

static ICON_PATH_CACHE: OnceLock<Mutex<IconPathCache>> = OnceLock::new();
const MAX_LOOKUP_CACHE_ENTRIES: usize = 2048;

pub fn resolve_icon_name_to_path(icon_name: &str, theme: &str) -> Option<String> {
    let cache = ICON_PATH_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (theme.to_owned(), icon_name.to_owned());
    if let Some(cached) = cache.lock().ok()?.get(&key).cloned() {
        return cached;
    }
    let result = resolve_icon_name_to_path_uncached(icon_name, theme);
    if let Ok(mut entries) = cache.lock() {
        if entries.len() >= MAX_LOOKUP_CACHE_ENTRIES {
            entries.clear();
        }
        entries.insert(key, result.clone());
    }
    result
}

pub fn clear_icon_path_cache() {
    if let Some(cache) = ICON_PATH_CACHE.get()
        && let Ok(mut entries) = cache.lock()
    {
        entries.clear();
    }
}

fn resolve_icon_name_to_path_uncached(icon_name: &str, theme: &str) -> Option<String> {
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

    resolve_icon_name_in_dirs(icon_name, theme, &search_dirs)
}

fn resolve_icon_name_in_dirs(
    icon_name: &str,
    theme: &str,
    search_dirs: &[PathBuf],
) -> Option<String> {
    let icon_lower = icon_name.to_lowercase();
    let target_names = [
        format!("{icon_name}.png"),
        format!("{icon_name}.svg"),
        format!("{icon_name}.xpm"),
        format!("{icon_lower}.png"),
        format!("{icon_lower}.svg"),
        format!("{icon_lower}.xpm"),
    ];
    for dir in search_dirs {
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
    if !theme.is_empty() {
        for dir in search_dirs {
            if !dir.exists() || dir.ends_with("pixmaps") {
                continue;
            }
            let theme_dir = dir.join(theme);
            if theme_dir.exists()
                && let Some(found) = find_common_icon_path(&theme_dir, &target_names)
            {
                return Some(found.to_string_lossy().into_owned());
            }
        }
    }
    for dir in search_dirs {
        if !dir.exists() || dir.ends_with("pixmaps") {
            continue;
        }
        let hicolor_dir = dir.join("hicolor");
        if hicolor_dir.exists()
            && let Some(found) = find_common_icon_path(&hicolor_dir, &target_names)
        {
            return Some(found.to_string_lossy().into_owned());
        }
    }
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

fn search_in_dir_recursive(
    dir: &Path,
    target_names: &[String],
    current_depth: usize,
    max_depth: usize,
) -> Option<PathBuf> {
    if current_depth > max_depth {
        return None;
    }
    let mut subdirs = Vec::new();
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if current_depth != 0 || path.file_name().is_none_or(|name| name != "hicolor") {
                subdirs.push(path);
            }
        } else if path.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| target_names.iter().any(|target| target == name))
        {
            return Some(path);
        }
    }
    subdirs.into_iter().find_map(|subdir| {
        search_in_dir_recursive(&subdir, target_names, current_depth + 1, max_depth)
    })
}

/// Finds the icon declared by an application's desktop entry.
pub fn find_fallback_icon(app_id: &str, theme: &str) -> Option<String> {
    if let Some(path) = crate::applications::find_desktop_file_path(app_id)
        && let Ok(entry) = DesktopEntry::from_path(&path, None::<&[&str]>)
        && let Some(icon) = entry.icon()
    {
        return Some(icon.to_string());
    }
    resolve_icon_name_to_path(app_id, theme)
}

#[cfg(test)]
mod tests {
    use super::resolve_icon_name_in_dirs;

    #[test]
    fn resolves_the_same_icon_from_the_selected_theme() {
        let root = std::env::temp_dir().join(format!(
            "cosmic-ext-niri-windows-icons-{}",
            std::process::id()
        ));
        let first = root.join("First/apps/scalable");
        let second = root.join("Second/apps/scalable");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(first.join("demo.svg"), "first").unwrap();
        std::fs::write(second.join("demo.svg"), "second").unwrap();

        let roots = [root.clone()];
        assert_eq!(
            resolve_icon_name_in_dirs("demo", "First", &roots),
            Some(first.join("demo.svg").to_string_lossy().into_owned())
        );
        assert_eq!(
            resolve_icon_name_in_dirs("demo", "Second", &roots),
            Some(second.join("demo.svg").to_string_lossy().into_owned())
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
