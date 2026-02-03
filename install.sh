#!/bin/sh
set -e

REPO="fairhill1/hal"
INSTALL_DIR="$HOME/.local/bin"

# Detect OS
OS=$(uname -s)
case "$OS" in
    Linux)  OS_NAME="linux" ;;
    Darwin) OS_NAME="macos" ;;
    MINGW*|MSYS*|CYGWIN*) OS_NAME="windows" ;;
    *) echo "Unsupported OS: $OS"; exit 1 ;;
esac

# Detect architecture
ARCH=$(uname -m)
case "$ARCH" in
    x86_64|amd64)  ARCH_NAME="x86_64" ;;
    aarch64|arm64) ARCH_NAME="aarch64" ;;
    *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac

# Build binary name
if [ "$OS_NAME" = "windows" ]; then
    BINARY="hal-${OS_NAME}-${ARCH_NAME}.exe"
else
    BINARY="hal-${OS_NAME}-${ARCH_NAME}"
fi

URL="https://github.com/${REPO}/releases/latest/download/${BINARY}"

echo "Downloading hal for ${OS_NAME}-${ARCH_NAME}..."

# Create install directory
mkdir -p "$INSTALL_DIR"

# Download
if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$URL" -o "${INSTALL_DIR}/hal"
elif command -v wget >/dev/null 2>&1; then
    wget -q "$URL" -O "${INSTALL_DIR}/hal"
else
    echo "Error: curl or wget is required"
    exit 1
fi

chmod +x "${INSTALL_DIR}/hal"

echo ""
echo "hal installed to ${INSTALL_DIR}/hal"
echo ""

# Check if install dir is in PATH
case ":$PATH:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
        echo "Add ${INSTALL_DIR} to your PATH:"
        echo ""
        echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
        echo ""
        echo "Add the line above to your ~/.bashrc or ~/.zshrc to make it permanent."
        echo ""
        ;;
esac

echo "Run 'hal' to get started"
