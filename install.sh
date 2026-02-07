#!/bin/bash
set -e

INSTALL_DIR="$HOME/.local/share/rustclaw"
SERVICE_NAME="rustclaw"

if [ -f "$INSTALL_DIR/rustclaw" ]; then
    echo "=== RustClaw Update Script ==="
    UPDATE=true
else
    echo "=== RustClaw Installation Script ==="
    UPDATE=false
fi

echo "[1/4] Building release binary..."
cargo build --release

if [ "$UPDATE" = true ]; then
    echo "[2/4] Stopping service..."
    systemctl --user stop "$SERVICE_NAME" 2>/dev/null || true
fi

echo "[3/4] Installing files..."
mkdir -p "$INSTALL_DIR/data"
mkdir -p "$HOME/.config/systemd/user"
cp target/release/rustclaw "$INSTALL_DIR/"
chmod 755 "$INSTALL_DIR/rustclaw"

if [ -f config.toml ] && [ "$UPDATE" = false ]; then
    cp config.toml "$INSTALL_DIR/"
    chmod 600 "$INSTALL_DIR/config.toml"
elif [ ! -f "$INSTALL_DIR/config.toml" ]; then
    echo "Warning: config.toml not found. Please create $INSTALL_DIR/config.toml manually."
fi

sed "s|%INSTALL_DIR%|$INSTALL_DIR|g" rustclaw.service > "$HOME/.config/systemd/user/$SERVICE_NAME.service"

echo "[4/4] Enabling service..."
systemctl --user daemon-reload
systemctl --user enable "$SERVICE_NAME"

if [ "$UPDATE" = true ]; then
    systemctl --user start "$SERVICE_NAME"
fi

echo ""
if [ "$UPDATE" = true ]; then
    echo "=== Update Complete ==="
else
    echo "=== Installation Complete ==="
fi
echo ""
echo "Commands:"
echo "  systemctl --user start $SERVICE_NAME    # Start"
echo "  systemctl --user stop $SERVICE_NAME     # Stop"
echo "  systemctl --user status $SERVICE_NAME   # Status"
echo "  journalctl --user -u $SERVICE_NAME -f   # Logs"
echo ""
echo "Enable linger for auto-start on boot:"
echo "  sudo loginctl enable-linger \$USER"
echo ""
echo "Config: $INSTALL_DIR/config.toml"
echo "Data:   $INSTALL_DIR/data/"
