#!/usr/bin/env sh

set -eu

BASE_URL="https://github.com/13/cronv/releases/latest/download"
INSTALL_DIR="/usr/local/bin"
TMP_DIR="$(mktemp -d)"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

# Detect OS
OS="$(uname -s)"
case "$OS" in
  Linux) OS="linux" ;;
  Darwin) OS="macos" ;;
  *)
    echo "Unsupported OS: $OS"
    exit 1
    ;;
esac

# Detect ARCH
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) ARCH="x86_64" ;;
  arm64|aarch64) ARCH="aarch64" ;;
  *)
    echo "Unsupported architecture: $ARCH"
    exit 1
    ;;
esac

# Select file
if [ "$OS" = "linux" ]; then
  FILE="cronv-linux-x86_64-musl.tar.gz"
else
  FILE="cronv-macos-${ARCH}.tar.gz"
fi

CHECKSUM_FILE="${FILE}.sha256"

echo "Downloading $FILE..."
curl -fsSL -o "$TMP_DIR/$FILE" "$BASE_URL/$FILE"

echo "Downloading checksum..."
curl -fsSL -o "$TMP_DIR/$CHECKSUM_FILE" "$BASE_URL/$CHECKSUM_FILE"

cd "$TMP_DIR"

# Detect checksum tool
if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL="$(sha256sum "$FILE" | cut -d ' ' -f1)"
elif command -v shasum >/dev/null 2>&1; then
  ACTUAL="$(shasum -a 256 "$FILE" | cut -d ' ' -f1)"
else
  echo "No SHA256 tool found"
  exit 1
fi

EXPECTED="$(cut -d ' ' -f1 < "$CHECKSUM_FILE")"

echo "Verifying checksum..."
if [ "$EXPECTED" != "$ACTUAL" ]; then
  echo "Checksum mismatch!"
  exit 1
fi

echo "Checksum OK"

echo "Extracting..."
tar xzf "$FILE"

echo "Installing..."
if [ -w "$INSTALL_DIR" ]; then
  mv cronv "$INSTALL_DIR/"
else
  sudo mv cronv "$INSTALL_DIR/"
fi

echo "Done! Run: cronv"
