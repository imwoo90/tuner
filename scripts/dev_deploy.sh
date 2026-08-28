#!/usr/bin/env bash
set -e

CONTAINER_NAME="tuner-sandbox"
PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "🔨 Building release binary..."
cargo build --release --manifest-path "$PROJECT_DIR/Cargo.toml"

RELEASE_BIN="$PROJECT_DIR/target/release/tuner"
DEFAULTS_DIR="$PROJECT_DIR/_home_defaults"

if [ ! -f "$RELEASE_BIN" ]; then
    echo "❌ Error: Release binary not found at $RELEASE_BIN"
    exit 1
fi

echo "📦 Syncing binary & _home_defaults asset to $CONTAINER_NAME..."

# Ensure target directory exists
docker exec "$CONTAINER_NAME" mkdir -p /root/.tuner/bin

# Copy binary atomically
docker cp "$RELEASE_BIN" "$CONTAINER_NAME:/root/.tuner/bin/tuner.new"
docker exec "$CONTAINER_NAME" mv -f /root/.tuner/bin/tuner.new /root/.tuner/bin/tuner
docker exec "$CONTAINER_NAME" chmod +x /root/.tuner/bin/tuner

# Copy _home_defaults assets
if [ -d "$DEFAULTS_DIR" ]; then
    docker cp "$DEFAULTS_DIR" "$CONTAINER_NAME:/root/.tuner/bin/"
fi

echo "🔄 Restarting container tuner worker..."
docker exec "$CONTAINER_NAME" pkill -9 -f tuner || true
docker exec -d "$CONTAINER_NAME" bash -c "while true; do /root/.tuner/bin/tuner --worker default; sleep 1; done"

echo "✅ Dev deploy complete! Container $CONTAINER_NAME is running updated binary & assets."
