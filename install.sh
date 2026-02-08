#!/bin/bash
set -e

REPO="shimaenaga1123/rustclaw"
BINARY_NAME="rustclaw"
SERVICE_NAME="rustclaw"

# ============================================================
# Linux: systemd user services
# ============================================================
setup_linux_service() {
    mkdir -p "$HOME/.config/systemd/user"

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
}

# ============================================================
# macOS: launchd agents
# ============================================================
setup_macos_service() {
    local AGENTS_DIR="$HOME/Library/LaunchAgents"
    local LOG_DIR="$HOME/Library/Logs/rustclaw"
    local PLIST_LABEL="com.rustclaw.bot"
    local UPDATE_LABEL="com.rustclaw.update"
    local UID_NUM
    UID_NUM=$(id -u)

    mkdir -p "$AGENTS_DIR"
    mkdir -p "$LOG_DIR"

    cat > "$AGENTS_DIR/${PLIST_LABEL}.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>${PLIST_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>${INSTALL_DIR}/${BINARY_NAME}</string>
    </array>
    <key>WorkingDirectory</key>
    <string>${INSTALL_DIR}</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>${LOG_DIR}/rustclaw.log</string>
    <key>StandardErrorPath</key>
    <string>${LOG_DIR}/rustclaw.err</string>
    <key>ThrottleInterval</key>
    <integer>5</integer>
</dict>
</plist>
EOF

    cat > "$AGENTS_DIR/${UPDATE_LABEL}.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>${UPDATE_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>/bin/bash</string>
        <string>-c</string>
        <string>${INSTALL_DIR}/${BINARY_NAME}-update &amp;&amp; launchctl kickstart -k gui/${UID_NUM}/${PLIST_LABEL}</string>
    </array>
    <key>WorkingDirectory</key>
    <string>${INSTALL_DIR}</string>
    <key>StartCalendarInterval</key>
    <dict>
        <key>Hour</key>
        <integer>4</integer>
        <key>Minute</key>
        <integer>0</integer>
    </dict>
    <key>StandardOutPath</key>
    <string>${LOG_DIR}/update.log</string>
    <key>StandardErrorPath</key>
    <string>${LOG_DIR}/update.err</string>
</dict>
</plist>
EOF

    launchctl bootstrap "gui/${UID_NUM}" "$AGENTS_DIR/${UPDATE_LABEL}.plist" 2>/dev/null || \
        launchctl load "$AGENTS_DIR/${UPDATE_LABEL}.plist" 2>/dev/null || true

    if [ "$RESTART" = true ]; then
        launchctl bootstrap "gui/${UID_NUM}" "$AGENTS_DIR/${PLIST_LABEL}.plist" 2>/dev/null || \
            launchctl load "$AGENTS_DIR/${PLIST_LABEL}.plist" 2>/dev/null || true
    fi

    echo ""
    echo "=== Installation Complete (${LATEST_TAG}) ==="
    echo ""
    echo "Commands:"
    echo "  launchctl kickstart gui/${UID_NUM}/${PLIST_LABEL}      # Start"
    echo "  launchctl kill SIGTERM gui/${UID_NUM}/${PLIST_LABEL}    # Stop"
    echo "  launchctl print gui/${UID_NUM}/${PLIST_LABEL}           # Status"
    echo "  tail -f ${LOG_DIR}/rustclaw.log                         # Logs"
    echo ""
    echo "Auto-update: daily at 04:00"
    echo "  launchctl kickstart gui/${UID_NUM}/${UPDATE_LABEL}      # Manual update"
    echo "  launchctl bootout gui/${UID_NUM}/${UPDATE_LABEL}        # Disable auto-update"
}

# ============================================================
# Main
# ============================================================
echo "=== RustClaw Installer ==="

# --- Detect OS and architecture ---
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux)  PLATFORM="linux" ;;
    Darwin) PLATFORM="macos" ;;
    *)      echo "Error: Unsupported OS: $OS"; exit 1 ;;
esac

case "$ARCH" in
    x86_64|amd64)   ARCH="x86_64" ;;
    arm64|aarch64)   ARCH="aarch64" ;;
    *)               echo "Error: Unsupported architecture: $ARCH"; exit 1 ;;
esac

case "${PLATFORM}-${ARCH}" in
    linux-x86_64)   TARGET="x86_64-unknown-linux-gnu" ;;
    macos-aarch64)  TARGET="aarch64-apple-darwin" ;;
    macos-x86_64)   TARGET="x86_64-apple-darwin" ;;
    *)              echo "Error: No prebuilt binary for ${PLATFORM}-${ARCH}"; exit 1 ;;
esac

echo "Detected: ${PLATFORM} ${ARCH} (${TARGET})"

# --- Paths ---
INSTALL_DIR="$HOME/.local/share/rustclaw"
BIN_DIR="$HOME/.local/bin"

# --- Current version ---
CURRENT_VERSION=""
if [ -f "$INSTALL_DIR/version" ]; then
    CURRENT_VERSION=$(cat "$INSTALL_DIR/version")
    echo "Current version: ${CURRENT_VERSION}"
fi

# --- Fetch latest release ---
echo "Fetching latest release..."
LATEST_TAG=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name"' | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')

if [ -z "$LATEST_TAG" ]; then
    echo "Error: Failed to fetch latest release."
    exit 1
fi

echo "Latest version: ${LATEST_TAG}"

if [ "$CURRENT_VERSION" = "$LATEST_TAG" ]; then
    echo "Already up to date."
    exit 0
fi

# --- Download ---
ASSET_NAME="${BINARY_NAME}-${TARGET}.tar.xz"
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${LATEST_TAG}/${ASSET_NAME}"

echo "[1/4] Downloading ${ASSET_NAME}..."
TMP_DIR=$(mktemp -d)
trap "rm -rf $TMP_DIR" EXIT

curl -fsSL -o "$TMP_DIR/$ASSET_NAME" "$DOWNLOAD_URL"

echo "[2/4] Extracting..."
tar -xJf "$TMP_DIR/$ASSET_NAME" -C "$TMP_DIR"

EXTRACT_DIR=$(find "$TMP_DIR" -mindepth 1 -maxdepth 1 -type d | head -1)
if [ -z "$EXTRACT_DIR" ]; then
    EXTRACT_DIR="$TMP_DIR"
fi

if [ ! -f "$EXTRACT_DIR/$BINARY_NAME" ]; then
    echo "Error: Binary not found in archive."
    exit 1
fi

# --- Stop existing service ---
echo "[3/4] Installing..."
RESTART=false

if [ "$PLATFORM" = "linux" ]; then
    if systemctl --user is-active "$SERVICE_NAME" &>/dev/null; then
        echo "Stopping service..."
        systemctl --user stop "$SERVICE_NAME"
        RESTART=true
    fi
elif [ "$PLATFORM" = "macos" ]; then
    if launchctl print "gui/$(id -u)/com.rustclaw.bot" &>/dev/null 2>&1; then
        echo "Stopping service..."
        launchctl bootout "gui/$(id -u)/com.rustclaw.bot" 2>/dev/null || true
        RESTART=true
    fi
fi

# --- Install binaries ---
mkdir -p "$INSTALL_DIR/data"
mkdir -p "$BIN_DIR"

cp "$EXTRACT_DIR/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
chmod 755 "$INSTALL_DIR/$BINARY_NAME"

if [ -f "$EXTRACT_DIR/${BINARY_NAME}-update" ]; then
    cp "$EXTRACT_DIR/${BINARY_NAME}-update" "$INSTALL_DIR/${BINARY_NAME}-update"
    chmod 755 "$INSTALL_DIR/${BINARY_NAME}-update"
fi

echo "$LATEST_TAG" > "$INSTALL_DIR/version"
ln -sf "$INSTALL_DIR/$BINARY_NAME" "$BIN_DIR/$BINARY_NAME"

# --- Config ---
if [ ! -f "$INSTALL_DIR/config.toml" ]; then
    if [ -f config.example.toml ]; then
        cp config.example.toml "$INSTALL_DIR/config.toml"
        chmod 600 "$INSTALL_DIR/config.toml"
        echo "Config template copied. Please edit: $INSTALL_DIR/config.toml"
    else
        echo "Warning: No config found. Please create $INSTALL_DIR/config.toml"
    fi
fi

# --- Setup service ---
echo "[4/4] Setting up service..."

if [ "$PLATFORM" = "linux" ]; then
    setup_linux_service
elif [ "$PLATFORM" = "macos" ]; then
    setup_macos_service
fi

echo ""
echo "Config: $INSTALL_DIR/config.toml"
echo "Data:   $INSTALL_DIR/data/"