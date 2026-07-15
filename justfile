# build the applet
build:
    cargo build --release

# run the applet standalone (for quick testing)
run:
    COSMIC_PANEL_SIZE=M COSMIC_PANEL_ANCHOR=Top COSMIC_PANEL_NAME=Panel cargo run --release

# install the applet locally to user directories (no sudo required)
install: build
    mkdir -p ~/.local/bin
    mkdir -p ~/.local/share/applications
    rm -f ~/.local/bin/cosmic-ext-niri-windows
    cp target/release/cosmic-ext-niri-windows ~/.local/bin/
    sed "s|Exec=cosmic-ext-niri-windows|Exec=/home/ton/.local/bin/cosmic-ext-niri-windows|" data/com.github.ton.CosmicExtNiriWindows.desktop > ~/.local/share/applications/com.github.ton.CosmicExtNiriWindows.desktop
    chmod +x ~/.local/share/applications/com.github.ton.CosmicExtNiriWindows.desktop

# add the applet to the cosmic panel center section
add-to-panel:
    #!/usr/bin/env bash
    CONFIG_DIR="$HOME/.config/cosmic/com.system76.CosmicPanel.Panel/v1"
    CURRENT=$(cat "$CONFIG_DIR/plugins_center" 2>/dev/null)
    if echo "$CURRENT" | grep -q "com.github.ton.CosmicExtNiriWindows"; then
        echo "Applet already in panel center."
    else
        echo 'Some(["com.system76.CosmicAppList","com.github.ton.CosmicExtNiriWindows",])' > "$CONFIG_DIR/plugins_center"
        echo "Added to panel center. Restart cosmic-panel to apply."
    fi

# restart cosmic-panel to reload applets
restart-panel:
    pkill -x cosmic-panel || true
    sleep 0.5
    cosmic-panel &

# check the applet
check:
    cargo check
