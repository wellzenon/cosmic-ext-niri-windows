# ✨VIBECODED✨ COSMIC Niri Windows Applet

https://github.com/user-attachments/assets/7fec42dc-a27e-4169-ab17-7553a277453a


`cosmic-ext-niri-windows` is a niri window list applet for the COSMIC Desktop panel (`cosmic-panel`). 

- **Monitor Output Filtering**: Shows only per-monitor windows, across all workspaces on that monitor.
- **Drag and Drop**: Dragging and dropping icons reorders or moves windows across workspaces.
- **Left Click**: Focuses the window.
- **Double Left Click**: Centers the window.
- **Middle Click**: Closes the window.
- **Right Click**: Opens Context Menu.
- **Scroll Wheel / Touchpad**: Switches workspaces.

- **Context Menu**:
  - **Show workspace names**: Toggle workspace name labels.
  - **Close Application**: Closes target window.
  - **Column Width**: Dynamic column width presets based on Nir 's `config.kdl` steps.
  - **Pin Application**: Toggle pinned shortcuts for favorite apps.


## Installation

### 1. Arch Linux (AUR)
Install the pre-compiled binary package directly from the AUR using your favorite AUR helper:
```bash
yay -S cosmic-ext-niri-windows-bin
```
*(or `paru -S cosmic-ext-niri-windows-bin`)*

### 2. Flatpak Bundle 
Download the `.flatpak` bundle from the latest [GitHub Release](https://github.com/wellzenon/cosmic-ext-niri-windows/releases) and install it:
```bash
flatpak install ./cosmic-ext-niri-windows.flatpak
```

### 3. Cargo 
Install directly from source using Cargo:
```bash
cargo install --git https://github.com/wellzenon/cosmic-ext-niri-windows
```

### 4. Building Locally

This project uses `just` for workflow automation.

```bash
# Build native binary
just build

# Test in COSMIC panel
just test

# Install locally to ~/.local/bin
just install

# Restart panel to apply changes
just restart-panel
```
