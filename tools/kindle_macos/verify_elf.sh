#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)"
PINS="$ROOT/tools/kindle_macos/PINNED.txt"
BIN_DIR="${1:-$ROOT/target/kindle-macos/release}"
MAX_GLIBC="$(sed -n 's/^max_glibc=//p' "$PINS" | tr -d '\r')"

if command -v llvm-readelf >/dev/null 2>&1; then
  READELF=llvm-readelf
elif command -v brew >/dev/null 2>&1 \
  && [ -x "$(brew --prefix llvm@22 2>/dev/null)/bin/llvm-readelf" ]; then
  READELF="$(brew --prefix llvm@22)/bin/llvm-readelf"
elif command -v readelf >/dev/null 2>&1; then
  READELF=readelf
else
  echo "verify_elf: llvm-readelf/readelf not found" >&2
  exit 1
fi

for name in ink-probe scribe-app scribe-lifecycle; do
  [ -s "$BIN_DIR/$name" ] || {
    echo "verify_elf: missing $BIN_DIR/$name" >&2
    exit 1
  }
  DESC="$(file "$BIN_DIR/$name")"
  case "$DESC" in
    *"ELF 32-bit"*ARM*EABI5*stripped*) ;;
    *) echo "verify_elf: unexpected $name: $DESC" >&2; exit 1 ;;
  esac
  FLAGS="$($READELF -h "$BIN_DIR/$name" | sed -n 's/^[[:space:]]*Flags:[[:space:]]*//p')"
  case "$FLAGS" in
    0x5000400*) ;;
    *)
      ATTRS="$($READELF -A "$BIN_DIR/$name" 2>/dev/null || true)"
      printf '%s\n' "$ATTRS" | grep -q 'ABI_VFP_args' \
        && printf '%s\n' "$ATTRS" | grep -q 'AAPCS VFP' \
        || {
          echo "verify_elf: $name is not ARM EABI5 hard-float (flags=$FLAGS)" >&2
          exit 1
        }
      ;;
  esac
done

for name in ink-probe scribe-app; do
  file "$BIN_DIR/$name" | grep -q 'statically linked' || {
    echo "verify_elf: $name must be static musl" >&2
    exit 1
  }
  if "$READELF" --version-info "$BIN_DIR/$name" 2>/dev/null | grep -q 'Name: GLIBC_'; then
    echo "verify_elf: $name unexpectedly requires glibc" >&2
    exit 1
  fi
done

LIFECYCLE="$BIN_DIR/scribe-lifecycle"
file "$LIFECYCLE" | grep -q 'dynamically linked' || {
  echo "verify_elf: scribe-lifecycle must remain dynamically linked for liblipc dlopen" >&2
  exit 1
}
PROGRAM_HEADERS="$($READELF -l "$LIFECYCLE")"
case "$PROGRAM_HEADERS" in
  *'/lib/ld-linux-armhf.so.3'*) ;;
  *) echo "verify_elf: scribe-lifecycle has the wrong interpreter" >&2; exit 1 ;;
esac

VERSIONS="$($READELF --version-info "$LIFECYCLE" \
  | sed -nE 's/.*Name: (GLIBC_[0-9.]+).*/\1/p' \
  | sort -Vu)"
[ -n "$VERSIONS" ] || {
  echo "verify_elf: scribe-lifecycle has no glibc version requirements" >&2
  exit 1
}
MAX_REQUIRED="$(printf '%s\n' "$VERSIONS" | tail -n 1 | sed 's/^GLIBC_//')"
HIGHEST="$(printf '%s\n%s\n' "$MAX_REQUIRED" "$MAX_GLIBC" | sort -V | tail -n 1)"
[ "$HIGHEST" = "$MAX_GLIBC" ] || {
  echo "verify_elf: lifecycle requires GLIBC_$MAX_REQUIRED, ceiling is GLIBC_$MAX_GLIBC" >&2
  exit 1
}

for name in ink-probe scribe-app scribe-lifecycle; do
  printf '%s  %s\n' "$(shasum -a 256 "$BIN_DIR/$name" | awk '{print $1}')" "$name"
done
echo "verify_elf: OK lifecycle_max_glibc=GLIBC_$MAX_REQUIRED ceiling=GLIBC_$MAX_GLIBC"
