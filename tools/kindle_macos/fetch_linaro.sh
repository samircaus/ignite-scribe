#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)"
PINS="$ROOT/tools/kindle_macos/PINNED.txt"
DEST_ROOT="${IGNITE_TOOLCHAIN_DIR:-$ROOT/target/toolchains}"

pin() {
  sed -n "s/^$1=//p" "$PINS" | tr -d '\r'
}

ARCHIVE_NAME="$(pin linaro_archive)"
TOOLCHAIN_NAME="$(pin linaro_root)"
URL="$(pin linaro_url)"
WANT_SHA="$(pin linaro_sha256)"
ARCHIVE="$DEST_ROOT/$ARCHIVE_NAME"
TOOLCHAIN="$DEST_ROOT/$TOOLCHAIN_NAME"

[[ "$WANT_SHA" =~ ^[0-9a-f]{64}$ ]] || {
  echo "fetch_linaro: malformed linaro_sha256 in $PINS" >&2
  exit 1
}

if [ -x "$TOOLCHAIN/bin/arm-linux-gnueabihf-gcc" ]; then
  printf '%s\n' "$TOOLCHAIN"
  exit 0
fi

mkdir -p "$DEST_ROOT"

if [ ! -f "$ARCHIVE" ]; then
  TMP_ARCHIVE="$(mktemp "$DEST_ROOT/.linaro-download.XXXXXX")"
  trap 'rm -f "$TMP_ARCHIVE"' EXIT
  curl -fL --retry 3 --output "$TMP_ARCHIVE" "$URL"
  HAVE_SHA="$(shasum -a 256 "$TMP_ARCHIVE" | awk '{print $1}')"
  [ "$HAVE_SHA" = "$WANT_SHA" ] || {
    echo "fetch_linaro: checksum mismatch have=$HAVE_SHA want=$WANT_SHA" >&2
    exit 1
  }
  mv "$TMP_ARCHIVE" "$ARCHIVE"
  trap - EXIT
else
  HAVE_SHA="$(shasum -a 256 "$ARCHIVE" | awk '{print $1}')"
  [ "$HAVE_SHA" = "$WANT_SHA" ] || {
    echo "fetch_linaro: cached checksum mismatch have=$HAVE_SHA want=$WANT_SHA" >&2
    exit 1
  }
fi

EXTRACT_DIR="$(mktemp -d "$DEST_ROOT/.linaro-extract.XXXXXX")"
trap 'rm -rf "$EXTRACT_DIR"' EXIT
tar -xJf "$ARCHIVE" -C "$EXTRACT_DIR"
[ -x "$EXTRACT_DIR/$TOOLCHAIN_NAME/bin/arm-linux-gnueabihf-gcc" ] || {
  echo "fetch_linaro: archive did not contain the expected toolchain" >&2
  exit 1
}
mv "$EXTRACT_DIR/$TOOLCHAIN_NAME" "$TOOLCHAIN"
rmdir "$EXTRACT_DIR"
trap - EXIT

printf '%s\n' "$TOOLCHAIN"
