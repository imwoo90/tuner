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

# Resolve container HOME path dynamically
CONTAINER_HOME="$(docker exec "$CONTAINER_NAME" bash -c 'echo -n "$HOME"')"

# Ensure target directory exists
docker exec "$CONTAINER_NAME" mkdir -p "$CONTAINER_HOME/.tuner/bin"

# Copy binary atomically
docker cp "$RELEASE_BIN" "$CONTAINER_NAME:$CONTAINER_HOME/.tuner/bin/tuner.new"
docker exec "$CONTAINER_NAME" mv -f "$CONTAINER_HOME/.tuner/bin/tuner.new" "$CONTAINER_HOME/.tuner/bin/tuner"
docker exec "$CONTAINER_NAME" chmod +x "$CONTAINER_HOME/.tuner/bin/tuner"

# Copy _home_defaults assets
if [ -d "$DEFAULTS_DIR" ]; then
    docker cp "$DEFAULTS_DIR" "$CONTAINER_NAME:$CONTAINER_HOME/.tuner/bin/"
fi

echo "🔄 Restarting container tuner worker..."
docker exec "$CONTAINER_NAME" pkill -9 -f tuner || true
docker exec -d "$CONTAINER_NAME" bash -c 'while true; do "$HOME/.tuner/bin/tuner" --worker default; sleep 1; done'

echo "✅ Dev deploy complete! Container $CONTAINER_NAME is running updated binary & assets."
