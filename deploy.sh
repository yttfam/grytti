#!/bin/bash
set -e

TARGET="x86_64-unknown-linux-musl"
HOST="cali@mista"
SSH_KEY="$HOME/.ssh/cali_net_rsa"
REMOTE_DIR="/opt/grytti"
SERVICE_NAME="grytti"

echo "==> Building for $TARGET..."
CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-musl-gcc \
cargo build --release --target "$TARGET"

BINARY="target/$TARGET/release/grytti"
SIZE=$(du -h "$BINARY" | cut -f1)
echo "==> Binary: $SIZE"

echo "==> Ensuring remote directory..."
ssh -i "$SSH_KEY" "$HOST" "sudo mkdir -p $REMOTE_DIR && sudo chown cali:cali $REMOTE_DIR"

echo "==> Deploying binary..."
scp -i "$SSH_KEY" "$BINARY" "$HOST:$REMOTE_DIR/grytti.new"
ssh -i "$SSH_KEY" "$HOST" "mv $REMOTE_DIR/grytti.new $REMOTE_DIR/grytti"

echo "==> Deploying config (if not present)..."
scp -i "$SSH_KEY" grytti.toml.example "$HOST:$REMOTE_DIR/grytti.toml.example"
ssh -i "$SSH_KEY" "$HOST" "test -f $REMOTE_DIR/grytti.toml || cp $REMOTE_DIR/grytti.toml.example $REMOTE_DIR/grytti.toml"

echo "==> Installing systemd service..."
ssh -i "$SSH_KEY" "$HOST" "sudo tee /etc/systemd/system/$SERVICE_NAME.service > /dev/null" <<'UNIT'
[Unit]
Description=grytti — PTY stream parser and Telegram bot
After=network.target mosquitto.service hermytt.service
Wants=hermytt.service

[Service]
Type=simple
User=cali
Group=cali
WorkingDirectory=/opt/grytti
ExecStart=/opt/grytti/grytti /opt/grytti/grytti.toml
Restart=on-failure
RestartSec=3
Environment=RUST_LOG=info,grytti=debug

# Hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=/opt/grytti
PrivateTmp=true

[Install]
WantedBy=multi-user.target
UNIT

echo "==> Enabling and restarting service..."
ssh -i "$SSH_KEY" "$HOST" "sudo systemctl daemon-reload && sudo systemctl enable $SERVICE_NAME && sudo systemctl restart $SERVICE_NAME"

echo "==> Verifying..."
sleep 2
ssh -i "$SSH_KEY" "$HOST" "sudo systemctl is-active $SERVICE_NAME && sudo journalctl -u $SERVICE_NAME --no-pager -n 5"

echo "==> Done."
