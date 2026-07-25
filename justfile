# Step 1: Build the applet for target: 'native' (default), 'flatpak', or 'aur'
build target="native":
    #!/usr/bin/env bash
    if [ "{{target}}" = "native" ]; then
        echo "==> Building native binary..."
        cargo build --release
    elif [ "{{target}}" = "flatpak" ]; then
        echo "==> Building Flatpak package..."
        flatpak-builder --force-clean --disable-rofiles-fuse --user --install build-dir io.github.wellzenon.CosmicExtNiriWindows.yml
    elif [ "{{target}}" = "aur" ]; then
        echo "==> Building AUR package..."
        makepkg -si --noconfirm
    else
        echo "Invalid build target: {{target}}. Use 'just build', 'just build flatpak', or 'just build aur'."
        exit 1
    fi

# install the applet locally to user directories (no sudo required)
install:
    mkdir -p ~/.local/bin
    mkdir -p ~/.local/share/applications
    mkdir -p ~/.local/share/metainfo
    mkdir -p ~/.local/share/icons/hicolor/scalable/apps
    rm -f ~/.local/bin/cosmic-ext-niri-windows
    cp target/release/cosmic-ext-niri-windows ~/.local/bin/
    sed "s|Exec=cosmic-ext-niri-windows|Exec=$HOME/.local/bin/cosmic-ext-niri-windows|" data/io.github.wellzenon.CosmicExtNiriWindows.desktop > ~/.local/share/applications/io.github.wellzenon.CosmicExtNiriWindows.desktop
    cp data/io.github.wellzenon.CosmicExtNiriWindows.metainfo.xml ~/.local/share/metainfo/
    cp data/io.github.wellzenon.CosmicExtNiriWindows.svg ~/.local/share/icons/hicolor/scalable/apps/
    chmod +x ~/.local/share/applications/io.github.wellzenon.CosmicExtNiriWindows.desktop


# Step 2: Test the built applet in the panel: 'flatpak' (default), 'aur', or 'native' (no rebuild)
test target="flatpak":
    #!/usr/bin/env bash
    if [ "{{target}}" = "flatpak" ]; then
        echo "==> Testing FLATPAK build in COSMIC panel..."
        rm -f ~/.local/share/applications/io.github.wellzenon.CosmicExtNiriWindows.desktop
        just restart-panel
    elif [ "{{target}}" = "aur" ] || [ "{{target}}" = "native" ]; then
        echo "==> Testing {{target}} build in COSMIC panel..."
        just restart-panel
    else
        echo "Invalid test target: {{target}}. Use 'just test', 'just test flatpak', ou 'just test aur'."
        exit 1
    fi

# Step 3: Bundle the release artifacts into dist/ (no rebuild)
bundle:
    #!/usr/bin/env bash
    mkdir -p dist
    echo "==> Generating Flatpak Bundle..."
    flatpak-builder --force-clean --repo=flatpak-repo build-dir io.github.wellzenon.CosmicExtNiriWindows.yml
    flatpak build-bundle flatpak-repo dist/cosmic-ext-niri-windows.flatpak io.github.wellzenon.CosmicExtNiriWindows
    echo "==> Generating Native Release Tarball..."
    tar -czvf dist/cosmic-ext-niri-windows-v0.1.0-x86_64.tar.gz target/release/cosmic-ext-niri-windows data/
    echo "==> Generating AUR .SRCINFO..."
    if command -v makepkg >/dev/null 2>&1; then makepkg --printsrcinfo > dist/.SRCINFO; cp PKGBUILD dist/; fi
    echo ""
    echo "==> All distribution bundles generated in dist/:"
    ls -lh dist/

# Run the applet standalone (for quick debugging)
run:
    COSMIC_PANEL_SIZE=M COSMIC_PANEL_ANCHOR=Top COSMIC_PANEL_NAME=Panel cargo run --release

# Restart cosmic-panel to reload applets
restart-panel:
    @pkill -x cosmic-panel || true
    @sleep 1.2
    @pgrep -x cosmic-panel >/dev/null || (cosmic-panel &)

# Add the applet to cosmic panel center section
add-to-panel:
    #!/usr/bin/env bash
    CONFIG_DIR="$HOME/.config/cosmic/com.system76.CosmicPanel.Panel/v1"
    CURRENT=$(cat "$CONFIG_DIR/plugins_center" 2>/dev/null)
    if echo "$CURRENT" | grep -q "io.github.wellzenon.CosmicExtNiriWindows"; then
        echo "Applet already in panel center."
    else
        echo 'Some(["com.system76.CosmicAppList","io.github.wellzenon.CosmicExtNiriWindows",])' > "$CONFIG_DIR/plugins_center"
        echo "Added to panel center. Restart cosmic-panel to apply."
    fi

# Check code correctness
check:
    cargo check
