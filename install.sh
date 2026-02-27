#!/usr/bin/env sh
# Installs OctoDocs to ~/.local/
#
# Usage:
#   curl -f https://raw.githubusercontent.com/elranu/octodocs/main/install.sh | sh
#
# Supported: Linux x86_64

set -eu

REPO="elranu/octodocs"
INSTALL_DIR="$HOME/.local/octodocs"
BIN_DIR="$HOME/.local/bin"

main() {
    platform="$(uname -s)"
    arch="$(uname -m)"

    case "$platform" in
        Linux) ;;
        Darwin)
            echo "On macOS, download the .dmg directly from:"
            echo "  https://github.com/$REPO/releases/latest"
            exit 1
            ;;
        *)
            echo "Unsupported platform: $platform"
            exit 1
            ;;
    esac

    case "$arch" in
        x86_64) ;;
        *)
            echo "Unsupported architecture: $arch"
            echo "Only x86_64 is currently available. Check releases manually:"
            echo "  https://github.com/$REPO/releases/latest"
            exit 1
            ;;
    esac

    if command -v curl >/dev/null 2>&1; then
        download() { command curl -fL "$@"; }
    elif command -v wget >/dev/null 2>&1; then
        download() { wget -O- "$@"; }
    else
        echo "Error: 'curl' or 'wget' is required to install OctoDocs"
        exit 1
    fi

    temp="$(mktemp -d)"
    # Ensure the temp dir is cleaned up on exit (success or error)
    trap 'rm -rf "$temp"' EXIT
    binary="$temp/octodocs-app"

    echo "Downloading OctoDocs..."
    download "https://github.com/$REPO/releases/latest/download/octodocs-linux-x86_64" > "$binary"

    echo "Installing to $INSTALL_DIR..."
    mkdir -p "$INSTALL_DIR"
    cp "$binary" "$INSTALL_DIR/octodocs-app"
    chmod +x "$INSTALL_DIR/octodocs-app"
    mkdir -p "$BIN_DIR"
    ln -sf "$INSTALL_DIR/octodocs-app" "$BIN_DIR/octodocs"

    echo ""
    if echo ":${PATH}:" | grep -q ":$BIN_DIR:"; then
        echo "OctoDocs installed successfully. Run with: octodocs"
    else
        echo "OctoDocs installed to $INSTALL_DIR"
        echo "To run it from your terminal, add ~/.local/bin to your PATH:"
        case "$SHELL" in
            *zsh)
                echo "  echo 'export PATH=\$HOME/.local/bin:\$PATH' >> ~/.zshrc && source ~/.zshrc"
                ;;
            *fish)
                echo "  fish_add_path -U \$HOME/.local/bin"
                ;;
            *)
                echo "  echo 'export PATH=\$HOME/.local/bin:\$PATH' >> ~/.bashrc && source ~/.bashrc"
                ;;
        esac
        echo ""
        echo "Or run directly: $BIN_DIR/octodocs"
    fi
}

main "$@"
