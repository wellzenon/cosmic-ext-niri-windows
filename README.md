# ✨VIBECODED✨ COSMIC Niri Windows Applet

https://github.com/user-attachments/assets/7fec42dc-a27e-4169-ab17-7553a277453a


`cosmic-ext-niri-windows` is a niri window list applet for the COSMIC Desktop panel (`cosmic-panel`). 

- **Monitor Output Filtering**: shows only per monitor windows, but all workspaces from that monitor.

- ==**Drag and Drop**==: Dragging and dropping icons will move the windows.
- **Left Click**: Focuses the window.
- **Double Left Click**: Centers the window.
- **Middle Click**: Closes the window.
- **Right Click**: Opens ==Context Menu==.
- **Scroll Wheel / Touchpad**: Changes workspaces.

- ==**Context Menu**==:
  - **Anywhere on the bar**:
    1. **Show workspaces names**: Toggle workspaces names visibility.
  - **Over an open window icon**:
    1. **Close application**
    2. **Column width**: Based on Niri's config column width steps.
    3. **Pin Application**: Toggles pinned/unpinned application state.


## Installation & Running

### Through Flathub

Search for cosmic ext niri windows

### Building locally

This project uses `just` as a runner.

#### Install Locally
Installs the binary to `~/.local/bin` and copies the `.desktop` launcher to `~/.local/share/applications/` (no root permissions required):
```bash
just install
```

##### Apply Changes
To restart the panel and reload the applet:
```bash
just restart-panel
```
