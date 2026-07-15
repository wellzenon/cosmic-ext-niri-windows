use std::path::PathBuf;
use freedesktop_desktop_entry::DesktopEntry;
use std::fs;

pub fn find_fallback_icon(app_id: &str) -> Option<String> {
    let dirs = vec![
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
        dirs::data_dir().map(|d| d.join("applications")).unwrap_or_default(),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
    ];

    let search_names = vec![
        format!("{}.desktop", app_id),
        format!("{}.desktop", app_id.to_lowercase()),
    ];

    for dir in dirs {
        if !dir.exists() {
            continue;
        }

        // Try exact match first
        for name in &search_names {
            let path = dir.join(name);
            if path.exists() {
                if let Ok(entry) = DesktopEntry::from_path(&path, None::<&[&str]>) {
                    if let Some(icon) = entry.icon() {
                        return Some(icon.to_string());
                    }
                }
            }
        }
        
        // Then try fuzzy matching
        if let Ok(entries) = fs::read_dir(&dir) {
            for e in entries.flatten() {
                if let Ok(file_name) = e.file_name().into_string() {
                    if file_name.starts_with(app_id) && file_name.ends_with(".desktop") {
                        let path = e.path();
                        if let Ok(entry) = DesktopEntry::from_path(&path, None::<&[&str]>) {
                            if let Some(icon) = entry.icon() {
                                return Some(icon.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    None
}
