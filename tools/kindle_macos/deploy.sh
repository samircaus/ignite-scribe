#!/bin/zsh
# Build scribe-app for the Kindle and install it over Wi-Fi (or USB), keeping the previous binary.
#
#   tools/kindle_macos/deploy.sh             # build, back up, install, verify
#   tools/kindle_macos/deploy.sh --no-build  # install the last build
#   tools/kindle_macos/deploy.sh --reader --restart  # build the feature-gated MuPDF reader
#
# Transport: Wi-Fi SSH (port 2222, Mac-only firewall rule, key $IGNITE_HOME/kindle_ed25519; set up
# per tools/kindle_macos/README.md), else the USB control socket /tmp/kindle-ssh.sock.
# Every install first copies the Kindle's notebook state and current binary to
# $IGNITE_HOME/backups/kindle/, then keeps the old binary on the device as bin/scribe-app.prev.
#
#   KINDLE_WIFI   the Kindle's Wi-Fi address (required for Wi-Fi; give it a DHCP reservation)
#   IGNITE_HOME   local key, known_hosts and backups (default ~/.ignite)
#   KINDLE_USB    USB networking address (default 192.168.15.244, the usual USBNetwork address)
set -eu

REPO="${0:A:h:h:h}"
IGNITE_HOME="${IGNITE_HOME:-$HOME/.ignite}"
KINDLE_WIFI="${KINDLE_WIFI:-}"
KINDLE_USB="${KINDLE_USB:-192.168.15.244}"
KEY="${KINDLE_KEY:-$IGNITE_HOME/kindle_ed25519}"
KNOWN="$IGNITE_HOME/known_hosts_kindle"
BACKUPS="$IGNITE_HOME/backups/kindle"
BIN="$REPO/target/kindle-macos-musl/armv7-unknown-linux-musleabihf/release/scribe-app"
APP=/mnt/us/scribe-app

build=1 restart=0 reader=0
for a in "$@"; do
  case $a in
    --no-build) build=0 ;;
    --restart) restart=1 ;;
    --reader) reader=1 ;;
    *) echo "usage: $0 [--no-build] [--restart] [--reader]" >&2; exit 2 ;;
  esac
done

kindle() {
  if [[ -n ${VIA_USB:-} ]]; then
    ssh -o ControlPath=/tmp/kindle-ssh.sock -o ControlMaster=no -o BatchMode=yes "root@$KINDLE_USB" "$@"
  elif [[ -n $KINDLE_WIFI ]]; then
    ssh -i "$KEY" -o IdentitiesOnly=yes -o UserKnownHostsFile="$KNOWN" -o BatchMode=yes \
      -o ConnectTimeout=6 -p 2222 "root@$KINDLE_WIFI" "$@"
  else
    return 1
  fi
}

if ! kindle true 2>/dev/null; then
  if [[ -S /tmp/kindle-ssh.sock ]]; then
    VIA_USB=1
    kindle true || { echo "Kindle unreachable (Wi-Fi and USB)" >&2; exit 1; }
  else
    echo "Kindle unreachable over Wi-Fi (set KINDLE_WIFI; open Ignite once after a reboot) and no USB session" >&2
    exit 1
  fi
fi
echo "transport: ${VIA_USB:+USB}${VIA_USB:-Wi-Fi $KINDLE_WIFI}"

if (( build )); then
  if (( reader )); then
    # MuPDF's Makefile needs ELF-aware ARM tools for embedded fonts and static archives.
    (cd "$REPO" && \
      LD="$REPO/tools/kindle_macos/mupdf-tools/arm-ld" \
      AR="$REPO/tools/kindle_macos/mupdf-tools/zig-ar" \
      RANLIB="$REPO/tools/kindle_macos/mupdf-tools/zig-ranlib" \
      CARGO_TARGET_DIR=target/kindle-macos-musl cargo +1.89.0 zigbuild -p scribe-app \
      --locked --release --target armv7-unknown-linux-musleabihf \
      --features 'scribe-app/restore_from_server,scribe-app/reader')
  else
    (cd "$REPO" && CARGO_TARGET_DIR=target/kindle-macos-musl cargo +1.89.0 zigbuild -p scribe-app \
      --locked --release --target armv7-unknown-linux-musleabihf --features scribe-app/restore_from_server)
  fi
fi
[[ -x $BIN ]] || { echo "missing build: $BIN" >&2; exit 1; }
want=$(shasum -a 256 "$BIN" | cut -d' ' -f1)
have=$(kindle "sha256sum $APP/bin/scribe-app | cut -d' ' -f1")
if [[ $want == "$have" ]]; then
  echo "already installed: ${want:0:16}"
else
  stamp=$(date +%Y%m%d-%H%M%S)
  mkdir -p "$BACKUPS"
  kindle "cd $APP && tar -cf - state bin/scribe-app \$(ls -d fb-polarity 2>/dev/null)" > "$BACKUPS/kindle-scribe-app-$stamp.tar"
  echo "backup: $BACKUPS/kindle-scribe-app-$stamp.tar"
  kindle "cat > $APP/bin/scribe-app.new && chmod 755 $APP/bin/scribe-app.new && sync" < "$BIN"
  got=$(kindle "sha256sum $APP/bin/scribe-app.new | cut -d' ' -f1")
  [[ $got == "$want" ]] || { echo "checksum mismatch after upload; nothing installed" >&2; kindle "rm -f $APP/bin/scribe-app.new"; exit 1; }
  kindle "cd $APP/bin && cp -p scribe-app scribe-app.prev && mv -f scribe-app.new scribe-app && sync"
  echo "installed: ${want:0:16} (previous kept as bin/scribe-app.prev)"
fi

if (( restart )); then
  # Experimental: ask the Kindle's app manager to stop and start Ignite.
  before=$(kindle "pidof scribe-app || true")
  kindle "lipc-set-prop com.lab126.appmgrd stop app://com.markrai.ignite; sleep 3; lipc-set-prop com.lab126.appmgrd start app://com.markrai.ignite" || true
  sleep 8
  after=$(kindle "pidof scribe-app || true")
  echo "scribe-app pid: ${before:-none} -> ${after:-none}"
else
  echo "Reopen Ignite on the Kindle to run the new build."
fi
