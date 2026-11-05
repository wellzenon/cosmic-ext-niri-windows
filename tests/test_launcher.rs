use cosmic_ext_niri_windows::launcher::{build_launch_command, resolve_desktop_launch};
use std::path::Path;

#[test]
fn test_desktop_exec_resolution_preserves_losange_shell_argument() {
    let path = std::env::temp_dir().join(format!(
        "cosmic-ext-niri-windows-launcher-{}.desktop",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "[Desktop Entry]\nType=Application\nName=Losange\nExec=sh -c \"/usr/bin/losange -o '%u'\"\nTerminal=false\n",
    )
    .expect("test desktop entry should be writable");

    let launch = resolve_desktop_launch(&path).expect("desktop entry should resolve");

    assert_eq!(launch.exec, "sh -c \"/usr/bin/losange -o '%u'\"");
    assert!(!launch.terminal);

    let command = build_launch_command("xyz.timtimtim.Losange", Some(&path))
        .expect("desktop entry should build a launch command");
    assert!(
        command.ends_with(&[
            "uwsm".to_string(),
            "app".to_string(),
            "--".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            "/usr/bin/losange -o '%u'".to_string(),
        ]),
        "unexpected launch command: {command:?}"
    );

    std::fs::remove_file(path).expect("test desktop entry should be removable");
}

#[test]
fn test_icon_resolution() {
    let theme = cosmic::config::icon_theme();
    let test_apps = vec![
        "org.neovim.nvim",
        "foot",
        "librewolf",
        "cosmic-term",
        "firefox",
        "org.gnome.Nautilus",
    ];
    for app in test_apps {
        let icon_spec = cosmic_ext_niri_windows::icons::find_fallback_icon(app, &theme);
        let icon_path = icon_spec.as_deref().and_then(|icon| {
            cosmic_ext_niri_windows::icons::resolve_icon_name_to_path(icon, &theme)
        });
        println!(
            "App: {:<20} | Spec: {:<20?} | Resolved Path: {:?}",
            app, icon_spec, icon_path
        );
    }

    if Path::new("/usr/share/applications/obsidian.desktop").is_file() {
        let icon_name = cosmic_ext_niri_windows::icons::find_fallback_icon("md.Obsidian", &theme)
            .expect("Obsidian desktop entry should declare an icon");
        let icon_path =
            cosmic_ext_niri_windows::icons::resolve_icon_name_to_path(&icon_name, &theme)
                .expect("Obsidian icon should resolve through the active theme");
        assert!(Path::new(&icon_path).is_absolute());
    }
}

#[test]
fn test_canonical_desktop_id() {
    let canonical = cosmic_ext_niri_windows::applications::resolve_canonical_desktop_id("nvim");
    println!(">>> CANONICAL ID FOR 'nvim': {:?}", canonical);
    assert_eq!(canonical, "org.neovim.nvim");
}

#[test]
fn test_webapp_id_is_not_collapsed_into_browser() {
    let app_id = "org.mozilla.firefox.webapp-39096a84-b8b4-429a-becc-b85fcb949875";
    assert_eq!(
        cosmic_ext_niri_windows::applications::resolve_canonical_desktop_id(app_id),
        app_id
    );
}
