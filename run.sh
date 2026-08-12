#!/bin/bash
cd "$(dirname "$0")"
echo "Compilation..."
cargo build --release 2>&1
if [ $? -eq 0 ]; then
    echo "Compiled. Starting..."
    # Force XWayland for drag-and-drop support on Wayland
    export WINIT_UNIX_BACKEND=x11
    if [ -n "$1" ]; then
        ./target/release/epubthing "$1"
    else
        ./target/release/epubthing
    fi
else
    echo "Compilation error"
    exit 1
fi
