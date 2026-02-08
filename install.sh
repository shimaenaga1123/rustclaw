#!/bin/bash
set -e

REPO="shimaenaga1123/rustclaw"
BINARY_NAME="rustclaw"
INSTALL_DIR="$HOME/.local/share/rustclaw"
BIN_DIR="$HOME/.local/bin"
SERVICE_NAME="rustclaw"

echo "=== RustClaw Installer ==="

# Detect current version
CURRENT_VERSION=""
if [ -f "$INSTALL_DIR/version" ]; then
    CURRENT_VERSION=$(cat "$INSTALL_DIR/version")
    echo "Current version: ${CURRENT_VERSION}"
fi

# Fetch latest release
echo "Fetching latest release..."
LATEST_TAG=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')

if [ -z "$LATEST_TAG" ]; then
    echo "Error: Failed to fetch latest release."
    exit 1
fi

echo "Latest version: ${LATEST_TAG}"

if [ "$CURRENT_VERSION" = "$LATEST_TAG" ]; then
    echo "Already up to date."
    exit 0
fi

# Download
ASSET_NAME="${BINARY_NAME}-x86_64-unknown-linux-gnu.tar.xz"
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${LATEST_TAG}/${ASSET_NAME}"

echo "[1/4] Downloading ${ASSET_NAME}..."
TMP_DIR=$(mktemp -d)
trap "rm -rf $TMP_DIR" EXIT

curl -fsSL -o "$TMP_DIR/$ASSET_NAME" "$DOWNLOAD_URL"

echo "[2/4] Extracting..."
tar -xJf "$TMP_DIR/$ASSET_NAME" -C "$TMP_DIR"

# cargo-dist extracts into a subdirectory
EXTRACT_DIR=$(find "$TMP_DIR" -mindepth 1 -maxdepth 1 -type d | head -1)
if [ -z "$EXTRACT_DIR" ]; then
    EXTRACT_DIR="$TMP_DIR"
fi

if [ ! -f "$EXTRACT_DIR/$BINARY_NAME" ]; then
    echo "Error: Binary not found in archive."
    exit 1
fi

# Stop service if running
echo "[3/4] Installing..."
if systemctl --user is-active "$SERVICE_NAME" &>/dev/null; then
    echo "Stopping service..."
    systemctl --user stop "$SERVICE_NAME"
    RESTART=true
else
    RESTART=false
fi

# Install binaries
mkdir -p "$INSTALL_DIR/data"
mkdir -p "$BIN_DIR"
mkdir -p "$HOME/.config/systemd/user"

cp "$EXTRACT_DIR/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
chmod 755 "$INSTALL_DIR/$BINARY_NAME"

# Install updater if present (shipped by cargo-dist)
if [ -f "$EXTRACT_DIR/${BINARY_NAME}-update" ]; then
    cp "$EXTRACT_DIR/${BINARY_NAME}-update" "$INSTALL_DIR/${BINARY_NAME}-update"
    chmod 755 "$INSTALL_DIR/${BINARY_NAME}-update"
fi

echo "$LATEST_TAG" > "$INSTALL_DIR/version"

# Symlink to PATH
ln -sf "$INSTALL_DIR/$BINARY_NAME" "$BIN_DIR/$BINARY_NAME"

# Config
if [ ! -f "$INSTALL_DIR/config.toml" ]; then
    if [ -f config.example.toml ]; then
        cp config.example.toml "$INSTALL_DIR/config.toml"
        chmod 600 "$INSTALL_DIR/config.toml"
        echo "Config template copied. Please edit: $INSTALL_DIR/config.toml"
    else
        echo "Warning: No config found. Please create $INSTALL_DIR/config.toml"
    fi
fi

# Systemd service
cat > "$HOME/.config/systemd/user/$SERVICE_NAME.service" << EOF
[Unit]
Description=RustClaw Discord Bot
Documentation=https://github.com/${REPO}
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=${INSTALL_DIR}
ExecStart=${INSTALL_DIR}/${BINARY_NAME}
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
EOF

# Auto-update service (uses cargo-dist's built-in updater)
cat > "$HOME/.config/systemd/user/${SERVICE_NAME}-update.service" << EOF
[Unit]
Description=RustClaw Auto-Update
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=${INSTALL_DIR}/${BINARY_NAME}-update
ExecStartPost=/usr/bin/systemctl --user restart ${SERVICE_NAME}
StandardOutput=journal
StandardError=journal
EOF

cat > "$HOME/.config/systemd/user/${SERVICE_NAME}-update.timer" << EOF
[Unit]
Description=RustClaw Auto-Update Timer

[Timer]
OnCalendar=*-*-* 04:00:00
RandomizedDelaySec=1800
Persistent=true

[Install]
WantedBy=timers.target
EOF

# Enable services
echo "[4/4] Enabling services..."
systemctl --user daemon-reload
systemctl --user enable "$SERVICE_NAME"
systemctl --user enable --now "${SERVICE_NAME}-update.timer"

if [ "$RESTART" = true ]; then
    systemctl --user start "$SERVICE_NAME"
fi

echo ""
echo "=== Installation Complete (${LATEST_TAG}) ==="
echo ""
echo "Commands:"
echo "  systemctl --user start $SERVICE_NAME      # Start"
echo "  systemctl --user stop $SERVICE_NAME       # Stop"
echo "  systemctl --user status $SERVICE_NAME     # Status"
echo "  journalctl --user -u $SERVICE_NAME -f     # Logs"
echo ""
echo "Auto-update: daily at 04:00 (±30min)"
echo "  systemctl --user status ${SERVICE_NAME}-update.timer  # Timer status"
echo "  systemctl --user start ${SERVICE_NAME}-update         # Manual update"
echo "  systemctl --user disable ${SERVICE_NAME}-update.timer # Disable auto-update"
echo ""
echo "Enable linger for auto-start on boot:"
echo "  sudo loginctl enable-linger \$USER"
echo ""
echo "Config: $INSTALL_DIR/config.toml"
echo "Data:   $INSTALL_DIR/data/"