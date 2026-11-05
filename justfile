# Validate the Rust code without producing release artifacts.
verify:
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-features

# Build, install and reload the applet for the selected distribution target.
test target="native":
    #!/usr/bin/env bash
    mkdir -p ~/.config/niri/scripts
    install -m 0755 scripts/cosmic-panel-guard ~/.config/niri/scripts/cosmic-panel-guard
    if [ "{{ target }}" = "native" ]; then
        echo "==> Building and installing native applet..."
        cargo build --release
        mkdir -p ~/.local/bin ~/.local/share/applications ~/.local/share/metainfo ~/.local/share/icons/hicolor/scalable/apps
        rm -f ~/.local/bin/cosmic-ext-niri-windows
        cp target/release/cosmic-ext-niri-windows ~/.local/bin/
        sed "s|Exec=cosmic-ext-niri-windows|Exec=$HOME/.local/bin/cosmic-ext-niri-windows|" data/io.github.wellzenon.CosmicExtNiriWindows.desktop > ~/.local/share/applications/io.github.wellzenon.CosmicExtNiriWindows.desktop
        cp data/io.github.wellzenon.CosmicExtNiriWindows.metainfo.xml ~/.local/share/metainfo/
        cp data/io.github.wellzenon.CosmicExtNiriWindows.svg ~/.local/share/icons/hicolor/scalable/apps/
        chmod +x ~/.local/share/applications/io.github.wellzenon.CosmicExtNiriWindows.desktop
    elif [ "{{ target }}" = "flatpak" ] || [ "{{ target }}" = "aur" ]; then
        echo "==> Reloading {{ target }} applet in COSMIC panel..."
        rm -f ~/.local/share/applications/io.github.wellzenon.CosmicExtNiriWindows.desktop
    else
        echo "Invalid target: {{ target }}. Use native, flatpak, or aur."
        exit 1
    fi
    if uwsm check is-active >/dev/null 2>&1; then
        panel_active_state="$(systemctl --user show cosmic-panel.service --property=ActiveState --value 2>/dev/null || true)"
        panel_pid="$(systemctl --user show cosmic-panel.service --property=MainPID --value 2>/dev/null || true)"
        panel_command="$(systemctl --user show cosmic-panel.service --property=ExecStart --value 2>/dev/null || true)"
        if [[ "$panel_command" == *"cosmic-panel-guard"* ]]; then
            systemctl --user restart cosmic-panel.service
        elif [ "$panel_active_state" = "active" ] && { [ -z "$panel_pid" ] || [ "$panel_pid" = "0" ]; }; then
            # A direct child left in an ExitType=cgroup service can keep the
            # unit active after the panel exits. Do not kill that cgroup.
            systemctl --user stop cosmic-panel-recovery.service 2>/dev/null || true
            uwsm app -t service -u cosmic-panel-recovery.service \
                -p Restart=on-failure -p RestartSec=1s -p ExitType=main -- \
                "$HOME/.config/niri/scripts/cosmic-panel-guard"
        else
            systemctl --user stop cosmic-panel.service 2>/dev/null || true
            uwsm app -t service -u cosmic-panel.service \
                -p Restart=on-failure -p RestartSec=1s -p ExitType=main -- \
                "$HOME/.config/niri/scripts/cosmic-panel-guard"
        fi
    else
        pkill -x cosmic-panel || true
        (cosmic-panel >/dev/null 2>&1 &)
    fi

# Produce Flatpak, native and AUR release artifacts in dist/.
bundle:
    #!/usr/bin/env bash
    cargo build --release
    mkdir -p dist
    echo "==> Generating Flatpak Bundle..."
    flatpak-builder --force-clean --repo=flatpak-repo build-dir io.github.wellzenon.CosmicExtNiriWindows.yml
    flatpak build-bundle flatpak-repo dist/cosmic-ext-niri-windows.flatpak io.github.wellzenon.CosmicExtNiriWindows
    echo "==> Generating Native Release Tarball..."
    tar -czvf dist/cosmic-ext-niri-windows-v0.1.0-x86_64.tar.gz target/release/cosmic-ext-niri-windows data/
    echo "==> Generating AUR .SRCINFO..."
    if command -v makepkg >/dev/null 2>&1; then makepkg --printsrcinfo > dist/.SRCINFO; cp PKGBUILD dist/; fi
    ls -lh dist/
