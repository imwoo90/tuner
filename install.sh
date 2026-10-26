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

# 6. Auto-install companion daemon agy-vault when Antigravity CLI is present
if [ "${TUNER_INSTALL_AGY_VAULT:-auto}" != "0" ] && { [ -d "$HOME/.gemini/antigravity-cli" ] || [ "${TUNER_INSTALL_AGY_VAULT:-auto}" = "1" ]; }; then
    echo "🗄️ Antigravity CLI detected. Installing companion daemon agy-vault..."
    VAULT_REPO="imwoo90/agy-vault"
    VAULT_REL=$(curl -s "https://api.github.com/repos/$VAULT_REPO/releases/latest" || true)
    VAULT_TAG=$(echo "$VAULT_REL" | grep -m 1 '"tag_name":' | awk -F '"' '{print $4}')
    if [ -n "$VAULT_TAG" ]; then
        VAULT_URL="https://github.com/$VAULT_REPO/releases/download/$VAULT_TAG/agy-vault-linux-amd64.tar.gz"
        VAULT_TAR="$TMP_DIR/agy-vault.tar.gz"
        if curl -fsSL -o "$VAULT_TAR" "$VAULT_URL"; then
            tar -xzf "$VAULT_TAR" -C "$TMP_DIR"
            cp "$TMP_DIR/agy-vault-linux-amd64/agy-vault" "$INSTALL_DIR/agy-vault"
            chmod +x "$INSTALL_DIR/agy-vault"
            if mkdir -p "$HOME/.local/bin" 2>/dev/null && [ -w "$HOME/.local/bin" ]; then
                ln -sf "$INSTALL_DIR/agy-vault" "$HOME/.local/bin/agy-vault" 2>/dev/null || true
            fi
            "$INSTALL_DIR/agy-vault" patch-binary || true
            echo "✅ agy-vault ($VAULT_TAG) installed to $INSTALL_DIR/agy-vault"
        else
            echo "⚠️ Warning: Could not download agy-vault package; skipping companion install."
        fi
    else
        echo "⚠️ Warning: Could not query agy-vault release tag; skipping companion install."
    fi
fi

# Cleanup temp files
rm -rf "$TMP_DIR"

echo "✅ tuner installed successfully to $INSTALL_DIR/tuner"
echo "💡 To configure tuner, please run: $INSTALL_DIR/tuner --setup"
