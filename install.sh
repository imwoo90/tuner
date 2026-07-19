#!/bin/bash
set -e

echo "🤖 Installing tuner..."

# 1. Detect target directory (defaults to ~/.tuner/bin)
INSTALL_DIR="${TUNER_INSTALL_DIR:-$HOME/.tuner/bin}"
mkdir -p "$INSTALL_DIR"

# 2. Fetch latest release information from GitHub
REPO="imwoo90/tuner"
echo "🔍 Fetching latest release info from GitHub..."
LATEST_RELEASE=$(curl -s "https://api.github.com/repos/$REPO/releases/latest")
TAG=$(echo "$LATEST_RELEASE" | grep -m 1 '"tag_name":' | awk -F '"' '{print $4}')

if [ -z "$TAG" ]; then
    echo "❌ Error: Could not retrieve latest release tag."
    exit 1
fi
echo "📦 Found latest release: $TAG"

# 3. Download the tarball package
DOWNLOAD_URL="https://github.com/$REPO/releases/download/$TAG/tuner-linux-amd64.tar.gz"
TMP_DIR=$(mktemp -d)
TARBALL="$TMP_DIR/tuner.tar.gz"

echo "📥 Downloading package from $DOWNLOAD_URL..."
curl -L -o "$TARBALL" "$DOWNLOAD_URL"

# 4. Extract package
echo "📦 Extracting package..."
tar -xzf "$TARBALL" -C "$TMP_DIR"

# 5. Copy files to install directory
echo "🚚 Copying files to $INSTALL_DIR..."
cp "$TMP_DIR/tuner-linux-amd64/tuner" "$INSTALL_DIR/tuner"
chmod +x "$INSTALL_DIR/tuner"

# Copy _home_defaults next to the binary
cp -r "$TMP_DIR/tuner-linux-amd64/_home_defaults" "$INSTALL_DIR/"

# Cleanup temp files
rm -rf "$TMP_DIR"

echo "✅ tuner installed successfully to $INSTALL_DIR/tuner"
echo "💡 To configure tuner, please run: $INSTALL_DIR/tuner --setup"
