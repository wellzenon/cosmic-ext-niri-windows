use cosmic_ext_niri_windows::launcher::SystemLauncher;
use std::path::Path;

#[test]
fn test_system_launcher_detection() {
    let launcher = SystemLauncher::detect();
    println!(">>> DETECTED SYSTEM LAUNCHER CAPABILITIES: {:?}", launcher);
    assert!(
        launcher.has_uwsm
            || launcher.has_gtk_launch
            || launcher.has_gio
            || launcher.default_terminal.is_some()
    );
}

#[test]
fn test_system_launcher_build_command() {
    let launcher = SystemLauncher::detect();
    let cmd = launcher.build_command_with_args(
        "org.neovim.nvim",
        Some(Path::new("/usr/share/applications/org.neovim.nvim.desktop")),
        true,
        Some(&["nvim".to_string()]),
    );
    println!(">>> BUILT COMMAND FOR NEOVIM: {:?}", cmd);
    assert!(!cmd.is_empty());
}

#[test]
fn test_launcher_keeps_exec_arguments_outside_a_shell() {
    let launcher = SystemLauncher {
        is_flatpak: false,
        has_uwsm: false,
        has_gtk_launch: false,
        has_gio: false,
        default_terminal: None,
    };
    let args = vec![
        "sh".to_string(),
        "-c".to_string(),
        "touch /tmp/pwned".to_string(),
    ];
    let command = launcher.build_command_with_args("demo", None, false, Some(&args));

    assert_eq!(command, args);
}

#[test]
fn test_empty_terminal_configuration_does_not_fall_back_to_shell() {
    let launcher = SystemLauncher {
        is_flatpak: false,
        has_uwsm: false,
        has_gtk_launch: false,
        has_gio: false,
        default_terminal: Some(String::new()),
    };
    let args = vec!["demo".to_string(), "--safe".to_string()];

    assert_eq!(
        launcher.build_command_with_args("demo", None, true, Some(&args)),
        args
    );
}

#[test]
fn test_icon_resolution() {
    let test_apps = vec![
        "org.neovim.nvim",
        "foot",
        "librewolf",
        "cosmic-term",
        "firefox",
        "org.gnome.Nautilus",
    ];
    for app in test_apps {
        let icon_spec = cosmic_ext_niri_windows::utils::find_fallback_icon(app);
        let icon_path = icon_spec
            .as_deref()
            .and_then(cosmic_ext_niri_windows::utils::resolve_icon_name_to_path);
        println!(
            "App: {:<20} | Spec: {:<20?} | Resolved Path: {:?}",
            app, icon_spec, icon_path
        );
    }

    if Path::new("/usr/share/applications/obsidian.desktop").is_file() {
        let icon_name = cosmic_ext_niri_windows::utils::find_fallback_icon("md.Obsidian")
            .expect("Obsidian desktop entry should declare an icon");
        let icon_path = cosmic_ext_niri_windows::utils::resolve_icon_name_to_path(&icon_name)
            .expect("Obsidian icon should resolve through the active theme");
        assert!(Path::new(&icon_path).is_absolute());
    }
}

#[test]
fn test_canonical_desktop_id() {
    let canonical = cosmic_ext_niri_windows::utils::resolve_canonical_desktop_id("nvim");
    println!(">>> CANONICAL ID FOR 'nvim': {:?}", canonical);
    assert_eq!(canonical, "org.neovim.nvim");
}

#[test]
fn test_webapp_id_is_not_collapsed_into_browser() {
    let app_id = "org.mozilla.firefox.webapp-39096a84-b8b4-429a-becc-b85fcb949875";
    assert_eq!(
        cosmic_ext_niri_windows::utils::resolve_canonical_desktop_id(app_id),
        app_id
    );
}
