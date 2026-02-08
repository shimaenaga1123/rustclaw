#!/bin/bash
set -e

BINARY_NAME="rustclaw"
SERVICE_NAME="rustclaw"
REPO="shimaenaga1123/rustclaw"
INSTALL_DIR="$HOME/.local/share/rustclaw"

OS="$(uname -s)"
case "$OS" in
    Linux)  PLATFORM="linux" ;;
    Darwin) PLATFORM="macos" ;;
    *)      echo "Error: Unsupported OS: $OS"; exit 1 ;;
esac

if [ ! -f "$INSTALL_DIR/$BINARY_NAME" ]; then
    echo "Error: $BINARY_NAME not found in $INSTALL_DIR"
    echo "Install the binary first:"
    echo "  curl --proto '=https' --tlsv1.2 -LsSf https://github.com/${REPO}/releases/latest/download/rustclaw-installer.sh | sh"
    exit 1
fi

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

    echo ""
    echo "=== Service Setup Complete ==="
    echo ""
    echo "Start the bot:"
    echo "  systemctl --user start $SERVICE_NAME"
    echo ""
    echo "Commands:"
    echo "  systemctl --user stop $SERVICE_NAME       # Stop"
    echo "  systemctl --user status $SERVICE_NAME     # Status"
    echo "  journalctl --user -u $SERVICE_NAME -f     # Logs"
    echo ""
    echo "Auto-update: daily at 04:00 (±30min)"
    echo "  systemctl --user status ${SERVICE_NAME}-update.timer  # Timer status"
    echo "  systemctl --user start ${SERVICE_NAME}-update         # Manual update"
    echo ""
    echo "Enable auto-start on boot:"
    echo "  sudo loginctl enable-linger \$USER"
}

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

    echo ""
    echo "=== Service Setup Complete ==="
    echo ""
    echo "Start the bot:"
    echo "  launchctl bootstrap gui/${UID_NUM} $AGENTS_DIR/${PLIST_LABEL}.plist"
    echo ""
    echo "Commands:"
    echo "  launchctl kill SIGTERM gui/${UID_NUM}/${PLIST_LABEL}    # Stop"
    echo "  launchctl kickstart -k gui/${UID_NUM}/${PLIST_LABEL}   # Restart"
    echo "  launchctl print gui/${UID_NUM}/${PLIST_LABEL}           # Status"
    echo "  tail -f ${LOG_DIR}/rustclaw.log                         # Logs"
    echo ""
    echo "Auto-update: daily at 04:00"
    echo "  launchctl kickstart gui/${UID_NUM}/${UPDATE_LABEL}      # Manual update"
}

mkdir -p "$INSTALL_DIR/data"

if [ ! -f "$INSTALL_DIR/config.toml" ]; then
    echo ""
    echo "Config not found. Please create: $INSTALL_DIR/config.toml"
    echo "  See: https://github.com/${REPO}#configuration"
fi

echo ""
echo "Config: $INSTALL_DIR/config.toml"
echo "Data:   $INSTALL_DIR/data/"

if [ "$PLATFORM" = "linux" ]; then
    setup_linux_service
elif [ "$PLATFORM" = "macos" ]; then
    setup_macos_service
fi
