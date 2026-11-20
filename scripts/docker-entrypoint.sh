#!/usr/bin/env bash
set -euo pipefail

export HOME="${HOME:-/home/tuner}"
TUNER_BIN_DIR="$HOME/.tuner/bin"
OPT_BIN_DIR="/opt/tuner/bin"

mkdir -p "$TUNER_BIN_DIR" "$HOME/.local/bin" "$HOME/.gemini"

ver_num() {
    printf '%s\n' "$1" | sed -E 's/^[^0-9]*([0-9]+)\.([0-9]+)\.([0-9]+).*/\1 \2 \3/' | awk '{printf "%05d%05d%05d\n", $1, $2, $3}'
}

seed_tuner=0
if [ ! -x "$TUNER_BIN_DIR/tuner" ]; then
    seed_tuner=1
elif [ -x "$OPT_BIN_DIR/tuner" ]; then
    img_ver="$("$OPT_BIN_DIR/tuner" --version 2>/dev/null | head -n 1 || true)"
    vol_ver="$("$TUNER_BIN_DIR/tuner" --version 2>/dev/null | head -n 1 || true)"
    if [ -n "$img_ver" ] && [ -n "$vol_ver" ]; then
        if [ "$(ver_num "$img_ver")" -gt "$(ver_num "$vol_ver")" ]; then
            seed_tuner=1
        fi
    fi
fi

if [ "$seed_tuner" -eq 1 ] && [ -x "$OPT_BIN_DIR/tuner" ]; then
    cp -f "$OPT_BIN_DIR/tuner" "$TUNER_BIN_DIR/tuner.tmp"
    chmod 0755 "$TUNER_BIN_DIR/tuner.tmp"
    mv -f "$TUNER_BIN_DIR/tuner.tmp" "$TUNER_BIN_DIR/tuner"
    if [ -d "$OPT_BIN_DIR/_home_defaults" ]; then
        rm -rf "$TUNER_BIN_DIR/_home_defaults"
        cp -r "$OPT_BIN_DIR/_home_defaults" "$TUNER_BIN_DIR/_home_defaults"
    fi
fi

if [ ! -d "$TUNER_BIN_DIR/_home_defaults" ] && [ -d "$OPT_BIN_DIR/_home_defaults" ]; then
    cp -r "$OPT_BIN_DIR/_home_defaults" "$TUNER_BIN_DIR/_home_defaults"
fi

if [ ! -x "$TUNER_BIN_DIR/cloudflared" ] && [ -x "$OPT_BIN_DIR/cloudflared" ]; then
    cp -f "$OPT_BIN_DIR/cloudflared" "$TUNER_BIN_DIR/cloudflared"
    chmod 0755 "$TUNER_BIN_DIR/cloudflared"
fi

exec "$TUNER_BIN_DIR/tuner" "$@"
