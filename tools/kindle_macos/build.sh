#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)"
PINS="$ROOT/tools/kindle_macos/PINNED.txt"

pin() {
  sed -n "s/^$1=//p" "$PINS" | tr -d '\r'
}

RUST_TOOLCHAIN="$(pin rust_toolchain)"
ZIG_VERSION="$(pin zig_version)"
CARGO_ZIGBUILD_VERSION="$(pin cargo_zigbuild_version)"
BUILDER_IMAGE="$(pin builder_image)"
BUILDER_TAG="$(pin builder_tag)"
MUSL_TARGET=armv7-unknown-linux-musleabihf
GNU_TARGET=armv7-unknown-linux-gnueabihf
MUSL_TARGET_DIR="$ROOT/target/kindle-macos-musl"
GNU_TARGET_DIR="$ROOT/target/kindle-macos-glibc217"
OUT="$ROOT/target/kindle-macos/release"

command -v zig >/dev/null 2>&1 || { echo "build: zig not found" >&2; exit 1; }
command -v cargo-zigbuild >/dev/null 2>&1 || { echo "build: cargo-zigbuild not found" >&2; exit 1; }
command -v docker >/dev/null 2>&1 || { echo "build: docker not found" >&2; exit 1; }

[ "$(zig version)" = "$ZIG_VERSION" ] || {
  echo "build: zig $(zig version) found, need $ZIG_VERSION" >&2
  exit 1
}
[ "$(cargo-zigbuild --version | awk '{print $2}')" = "$CARGO_ZIGBUILD_VERSION" ] || {
  echo "build: cargo-zigbuild version mismatch (need $CARGO_ZIGBUILD_VERSION)" >&2
  exit 1
}

INSTALLED_TOOLCHAINS="$(rustup toolchain list)"
case "$INSTALLED_TOOLCHAINS" in
  *"$RUST_TOOLCHAIN"*) ;;
  *) rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal >/dev/null ;;
esac
INSTALLED_TARGETS="$(rustup target list --toolchain "$RUST_TOOLCHAIN" --installed)"
for target in "$MUSL_TARGET" "$GNU_TARGET"; do
  printf '%s\n' "$INSTALLED_TARGETS" | grep -qx "$target" \
    || rustup target add --toolchain "$RUST_TOOLCHAIN" "$target" >/dev/null
done

TOOLCHAIN="$($ROOT/tools/kindle_macos/fetch_linaro.sh)"
TC_REL="${TOOLCHAIN#"$ROOT"/}"
case "$TC_REL" in
  "$TOOLCHAIN"|"")
    echo "build: toolchain must be under the repository for the container mount" >&2
    exit 1
    ;;
esac

export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }--remap-path-prefix=$ROOT=. --remap-path-prefix=$HOME/.cargo/registry=deps --remap-path-prefix=$HOME/.cargo/git=deps"

echo "== native macOS static musl build"
CARGO_TARGET_DIR="$MUSL_TARGET_DIR" cargo "+$RUST_TOOLCHAIN" zigbuild \
  -p ink-probe -p scribe-app \
  --locked --release --target "$MUSL_TARGET" \
  --features scribe-app/restore_from_server

DOCKER_CFG="$(mktemp -d)"
trap 'rm -rf "$DOCKER_CFG"' EXIT
DOCKER_ARGS=()
if [ -n "${IGNITE_DOCKER_HOST:-}" ]; then
  DOCKER_ARGS=(-H "$IGNITE_DOCKER_HOST")
elif [ -S "$HOME/.colima/default/docker.sock" ]; then
  DOCKER_ARGS=(-H "unix://$HOME/.colima/default/docker.sock")
fi

docker_cmd() {
  DOCKER_CONFIG="$DOCKER_CFG" docker "${DOCKER_ARGS[@]}" "$@"
}

docker_cmd info >/dev/null 2>&1 || {
  echo "build: Docker is unavailable; start Colima with the repository volume mounted" >&2
  exit 1
}

if ! docker_cmd image inspect "$BUILDER_TAG" >/dev/null 2>&1; then
  echo "== build pinned lifecycle compatibility image"
  docker_cmd build --platform linux/amd64 \
    --build-arg "BUILDER_IMAGE=$BUILDER_IMAGE" \
    -f "$ROOT/tools/kindle_macos/Dockerfile.glibc217" \
    -t "$BUILDER_TAG" "$ROOT/tools/kindle_macos"
fi

echo "== cached EGLIBC 2.17 lifecycle build"
docker_cmd run --rm --platform linux/amd64 \
  -v "$ROOT:/src" -w /src \
  -v ignite-kindle-cargo-registry:/usr/local/cargo/registry \
  "$BUILDER_TAG" bash -c "
set -euo pipefail
TC=/src/$TC_REL
export RUSTFLAGS='--remap-path-prefix=/src=. --remap-path-prefix=/usr/local/cargo/registry=deps --remap-path-prefix=/usr/local/cargo/git=deps -C link-arg=-march=armv7-a -C link-arg=-mfloat-abi=hard -C link-arg=-mfpu=neon-vfpv4'
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=\"\$TC/bin/arm-linux-gnueabihf-gcc\" \\
CARGO_TARGET_DIR=/src/${GNU_TARGET_DIR#"$ROOT"/} \\
cargo build -p scribe-lifecycle --locked --release --target $GNU_TARGET
"

mkdir -p "$OUT"
cp -f "$MUSL_TARGET_DIR/$MUSL_TARGET/release/ink-probe" "$OUT/ink-probe"
cp -f "$MUSL_TARGET_DIR/$MUSL_TARGET/release/scribe-app" "$OUT/scribe-app"
cp -f "$GNU_TARGET_DIR/$GNU_TARGET/release/scribe-lifecycle" "$OUT/scribe-lifecycle"
chmod 0755 "$OUT/ink-probe" "$OUT/scribe-app" "$OUT/scribe-lifecycle"

"$ROOT/tools/kindle_macos/verify_elf.sh" "$OUT"
echo "build: OK $OUT"
