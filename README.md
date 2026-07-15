# ✨VIBECODED✨ COSMIC Niri Windows Applet

`cosmic-ext-niri-windows` is a niri window list applet for the COSMIC Desktop panel (`cosmic-panel`). 

- **Monitor Output Filtering**: shows only per monitor windows, but all workspaces from that monitor.
- **Mouse Interactions**:
  - **Left Click**: Focuses the window.
  - **Middle Click**: Closes the window.
  - **Scroll Wheel / Touchpad**: Changes workspaces.

## Installation & Running

This project uses `just` as a runner.

### Install Locally
Installs the binary to `~/.local/bin` and copies the `.desktop` launcher to `~/.local/share/applications/` (no root permissions required):
```bash
just install
```

### Apply Changes
To restart the panel and reload the applet:
```bash
just restart-panel
```
