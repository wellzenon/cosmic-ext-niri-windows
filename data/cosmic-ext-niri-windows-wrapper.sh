#!/bin/bash
# Wrapper script for Flatpak sandbox.
# The Niri IPC socket at $XDG_RUNTIME_DIR/niri.wayland-*.sock cannot be
# accessed through Flatpak's xdg-run glob patterns because Flatpak treats
# them as literal directory names. Instead, we request access to the full
# /run/user/$UID directory at launch time via flatpak override, and this
# wrapper ensures the NIRI_SOCKET variable is set correctly.

# If NIRI_SOCKET is already set and the socket exists, just run the binary.
if [ -S "$NIRI_SOCKET" ]; then
    exec /app/bin/cosmic-ext-niri-windows "$@"
fi

# Try to find the niri socket by scanning XDG_RUNTIME_DIR
if [ -n "$XDG_RUNTIME_DIR" ]; then
    for sock in "$XDG_RUNTIME_DIR"/niri.wayland-*.sock; do
        if [ -S "$sock" ]; then
            export NIRI_SOCKET="$sock"
            exec /app/bin/cosmic-ext-niri-windows "$@"
        fi
    done
fi

# Fallback: just run the binary and let it handle the error
exec /app/bin/cosmic-ext-niri-windows "$@"
