#!/usr/bin/env sh
# OctoDocs installer — Linux x86_64 + macOS aarch64
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/elranu/octodocs/main/install.sh | sh
#
# After install:
#   octodocs            — launch the app
#   octodocs --version  — print installed version
#   octodocs --update   — update to the latest release

set -eu

REPO="elranu/octodocs"
INSTALL_DIR="$HOME/.local/share/octodocs"
BIN_DIR="$HOME/.local/bin"

# ─── helpers ──────────────────────────────────────────────────────────────────

say()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
ok()   { printf '\033[1;32m  ✓\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

if command -v curl >/dev/null 2>&1; then
    download()          { curl -fsSL "$@"; }
    download_progress() { curl -fL --progress-bar "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
    download()          { wget -qO- "$@"; }
    download_progress() { wget -q --show-progress -O "$2" "$1"; }
else
    die "'curl' or 'wget' is required to install OctoDocs"
fi

# ─── platform detection ───────────────────────────────────────────────────────

platform="$(uname -s)"
arch="$(uname -m)"

case "$platform-$arch" in
    Linux-x86_64)
        TARBALL_NAME="octodocs-linux-x86_64.tar.gz"
        ;;
    Darwin-arm64|Darwin-aarch64)
        TARBALL_NAME="octodocs-macos-aarch64.tar.gz"
        ;;
    Darwin-x86_64)
        die "macOS x86_64 builds are not yet available. Download from: https://github.com/$REPO/releases/latest"
        ;;
    *)
        die "Unsupported platform: $platform $arch. Check: https://github.com/$REPO/releases/latest"
        ;;
esac

# ─── version detection ────────────────────────────────────────────────────────

latest_version() {
    download "https://api.github.com/repos/$REPO/releases/latest" \
        | grep '"tag_name"' \
        | sed 's/.*"tag_name": *"\(.*\)".*/\1/'
}

VERSION_FILE="$INSTALL_DIR/VERSION"
INSTALLED_VERSION=""
[ -f "$VERSION_FILE" ] && INSTALLED_VERSION="$(cat "$VERSION_FILE")"

say "Checking latest version..."
VERSION="$(latest_version)"
[ -n "$VERSION" ] || die "Could not determine latest version — check your internet connection."

if [ -n "$INSTALLED_VERSION" ] && [ "$INSTALLED_VERSION" = "$VERSION" ]; then
    ok "OctoDocs $VERSION is already up to date."
    exit 0
fi

if [ -n "$INSTALLED_VERSION" ]; then
    say "Updating OctoDocs $INSTALLED_VERSION → $VERSION"
else
    say "Installing OctoDocs $VERSION"
fi

# ─── download ────────────────────────────────────────────────────────────────

temp="$(mktemp -d)"
trap 'rm -rf "$temp"' EXIT
tarball="$temp/octodocs.tar.gz"

say "Downloading..."
download_progress \
    "https://github.com/$REPO/releases/latest/download/$TARBALL_NAME" \
    "$tarball"

say "Extracting..."
tar -xzf "$tarball" -C "$temp"

# ─── install binary ──────────────────────────────────────────────────────────

mkdir -p "$INSTALL_DIR" "$BIN_DIR"
cp "$temp/octodocs-app" "$INSTALL_DIR/octodocs-app"
chmod +x "$INSTALL_DIR/octodocs-app"

# Copy assets (SVG toolbar icons) so the app can find them at runtime
if [ -d "$temp/assets" ]; then
    rm -rf "$INSTALL_DIR/assets"
    cp -r "$temp/assets" "$INSTALL_DIR/assets"
fi

printf '%s\n' "$VERSION" > "$VERSION_FILE"

# ─── install icon ─────────────────────────────────────────────────────────────

ICON_DIR="$HOME/.local/share/icons/hicolor/scalable/apps"
mkdir -p "$ICON_DIR"
download "https://raw.githubusercontent.com/elranu/octodocs/main/assets/octoDocs-icon.svg" \
    > "$ICON_DIR/octodocs.svg" 2>/dev/null || true

# ─── wrapper script ───────────────────────────────────────────────────────────
# Thin shell wrapper so --version and --update work without the GUI app handling args.

cat > "$BIN_DIR/octodocs" << 'WRAPPER'
#!/usr/bin/env sh
INSTALL_DIR="$HOME/.local/share/octodocs"
VERSION_FILE="$INSTALL_DIR/VERSION"
case "${1:-}" in
  --version|-v|version)
    if [ -f "$VERSION_FILE" ]; then
      printf 'OctoDocs %s\n' "$(cat "$VERSION_FILE")"
    else
      printf 'OctoDocs (version unknown)\n'
    fi
    ;;
  --update|update)
    printf 'Updating OctoDocs...\n'
    curl -fsSL https://raw.githubusercontent.com/elranu/octodocs/main/install.sh | sh
    ;;
  *)
    exec "$INSTALL_DIR/octodocs-app" "$@"
    ;;
esac
WRAPPER
chmod +x "$BIN_DIR/octodocs"

# ─── platform-specific integration ────────────────────────────────────────────

case "$platform" in
    Linux)
        # .desktop entry — picked up by GNOME, KDE, XFCE and other XDG desktops
        DESKTOP_DIR="$HOME/.local/share/applications"
        mkdir -p "$DESKTOP_DIR"
        cat > "$DESKTOP_DIR/octodocs.desktop" << DESKTOP
[Desktop Entry]
Name=OctoDocs
Comment=Markdown editor with GitHub sync
Exec=$BIN_DIR/octodocs %F
Icon=octodocs
Type=Application
Categories=Office;TextEditor;
MimeType=text/markdown;text/plain;
StartupNotify=true
StartupWMClass=octodocs
Terminal=false
Keywords=markdown;editor;notes;github;docs;
DESKTOP
        # Refresh caches so GNOME/KDE pick up the new entry immediately
        if command -v update-desktop-database >/dev/null 2>&1; then
            update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
        fi
        if command -v gtk-update-icon-cache >/dev/null 2>&1; then
            gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
        fi
        ok "Desktop shortcut created — check your app launcher"
        ;;

    Darwin)
        # Minimal .app bundle in ~/Applications so Spotlight and Finder work
        APP_CONTENTS="$HOME/Applications/OctoDocs.app/Contents"
        mkdir -p "$APP_CONTENTS/MacOS"
        cp "$INSTALL_DIR/octodocs-app" "$APP_CONTENTS/MacOS/octodocs"
        chmod +x "$APP_CONTENTS/MacOS/octodocs"
        # Copy icons so they're resolvable relative to the executable inside the bundle
        if [ -d "$INSTALL_DIR/assets" ]; then
            cp -r "$INSTALL_DIR/assets" "$APP_CONTENTS/MacOS/assets"
        fi
        cat > "$APP_CONTENTS/Info.plist" << 'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>octodocs</string>
    <key>CFBundleIdentifier</key><string>io.github.elranu.octodocs</string>
    <key>CFBundleName</key><string>OctoDocs</string>
    <key>CFBundleDisplayName</key><string>OctoDocs</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
</dict>
</plist>
PLIST
        ok "App bundle created at ~/Applications/OctoDocs.app (open via Spotlight or Finder)"
        ;;
esac

# ─── done ─────────────────────────────────────────────────────────────────────

echo ""
if echo ":${PATH}:" | grep -q ":$BIN_DIR:"; then
    ok "OctoDocs $VERSION installed successfully."
    echo ""
    echo "  octodocs            — launch"
    echo "  octodocs --version  — show installed version"
    echo "  octodocs --update   — update to latest"
else
    ok "OctoDocs $VERSION installed to $INSTALL_DIR"
    echo ""
    echo "  Add ~/.local/bin to your PATH to use the 'octodocs' command:"
    case "${SHELL:-}" in
        *zsh)  echo "    echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc && source ~/.zshrc" ;;
        *fish) echo "    fish_add_path -U \$HOME/.local/bin" ;;
        *)     echo "    echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc && source ~/.bashrc" ;;
    esac
    echo ""
    echo "  Or launch directly: $BIN_DIR/octodocs"
fi
echo ""
