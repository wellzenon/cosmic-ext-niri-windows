use cosmic_ext_niri_windows::launcher::SystemLauncher;
use std::path::Path;

#[test]
fn test_system_launcher_detection() {
    let launcher = SystemLauncher::detect();
    println!(">>> DETECTED SYSTEM LAUNCHER CAPABILITIES: {:?}", launcher);
    assert!(launcher.has_uwsm || launcher.has_gtk_launch || launcher.has_gio || launcher.default_terminal.is_some());
}

#[test]
fn test_system_launcher_build_command() {
    let launcher = SystemLauncher::detect();
    let cmd = launcher.build_command("org.neovim.nvim", Some(Path::new("/usr/share/applications/org.neovim.nvim.desktop")), true, Some("nvim"));
    println!(">>> BUILT COMMAND FOR NEOVIM: {:?}", cmd);
    assert!(!cmd.is_empty());
}

#[test]
fn test_icon_resolution() {
    let test_apps = vec!["org.neovim.nvim", "foot", "librewolf", "cosmic-term", "firefox", "org.gnome.Nautilus"];
    for app in test_apps {
        let icon_spec = cosmic_ext_niri_windows::utils::find_fallback_icon(app);
        let icon_path = icon_spec.as_deref().and_then(cosmic_ext_niri_windows::utils::resolve_icon_name_to_path);
        println!("App: {:<20} | Spec: {:<20?} | Resolved Path: {:?}", app, icon_spec, icon_path);
    }
}

#[test]
fn test_canonical_desktop_id() {
    let canonical = cosmic_ext_niri_windows::utils::resolve_canonical_desktop_id("nvim");
    println!(">>> CANONICAL ID FOR 'nvim': {:?}", canonical);
    assert_eq!(canonical, "org.neovim.nvim");
}
