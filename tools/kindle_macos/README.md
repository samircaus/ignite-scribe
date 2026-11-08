# Fast Kindle builds on macOS

This developer path keeps frequent `scribe-app` and `ink-probe` builds native
to macOS while preserving the dynamically loaded Kindle LIPC lifecycle host.

- `scribe-app` and `ink-probe`: static ARMv7 hard-float musl via Zig.
- `scribe-lifecycle`: ARMv7 hard-float EGLIBC 2.17 via the pinned Linaro
  2013.07 sysroot and a small cached compatibility container.
- `verify_elf.sh`: refuses wrong architecture, ABI, linkage, interpreter, or a
  lifecycle glibc requirement newer than the Kindle 5.18.3 ceiling (2.18).

Prerequisites:

```bash
brew install zig llvm@22
cargo install --locked cargo-zigbuild
colima start --runtime docker --mount "$PWD":w   # mount the checkout's volume
```

Build:

```bash
bash tools/kindle_macos/build.sh
```

The first run downloads and verifies the official Linaro toolchain and builds
the compatibility image. Later app/probe builds are native and incremental;
the small lifecycle build reuses Docker and Cargo caches. Outputs are staged at
`target/kindle-macos/release/`.

This is a developer/hardware-qualification path. The existing digest-pinned
Docker release pipeline remains the official packaging path until this path is
promoted with equivalent CI and release provenance.

## Installing over Wi-Fi (`deploy.sh`)

```bash
KINDLE_WIFI=192.168.1.30 tools/kindle_macos/deploy.sh            # build, back up, install, verify
KINDLE_WIFI=192.168.1.30 tools/kindle_macos/deploy.sh --restart  # also restart Ignite (experimental)
tools/kindle_macos/deploy.sh --reader                            # build with the MuPDF reader
```

Before every install the script copies the Kindle's notebook state and current binary to
`$IGNITE_HOME/backups/kindle/` (default `~/.ignite`), checks the upload's sha256, and keeps the old
binary on the device as `bin/scribe-app.prev`. Without Wi-Fi SSH it falls back to a USB SSH control
socket at `/tmp/kindle-ssh.sock`.

One-time setup over USB (requires a jailbroken Kindle with jobman's dropbear):

1. Make a key on the Mac: `ssh-keygen -t ed25519 -f ~/.ignite/kindle_ed25519 -N ''`.
2. Append `~/.ignite/kindle_ed25519.pub` to `/mnt/us/jobman/ssh/etc/dropbear/authorized_keys` on the
   Kindle. (Dropbear refuses `/mnt/us/.ssh` because `/mnt/us` is world-writable.)
3. Copy [`wifi-ssh/start.sh`](wifi-ssh/start.sh) to `/var/local/ignite-ssh/start.sh` (root, mode
   700) and set `MAC=` to the Mac's fixed address. It opens TCP 2222 on `wlan0` **for that one
   address only** and runs a second, key-only dropbear there; jobman's port 22 stays closed on Wi-Fi.
4. Start it on every Ignite launch: add
   `[ -x /var/local/ignite-ssh/start.sh ] && /var/local/ignite-ssh/start.sh >/dev/null 2>&1 &`
   to `/mnt/us/scribe-app/bin/scribe-lifecycle-host.sh` (keep a copy of the original). After a
   Kindle reboot, Wi-Fi SSH comes back once Ignite has been opened.
5. Give the Kindle a DHCP reservation too, and pass its address as `KINDLE_WIFI`.

Undo: remove that line, `kill $(cat /var/local/ignite-ssh/dropbear.pid)`, delete the iptables rule
(`iptables -D INPUT -i wlan0 -p tcp -s <mac-ip> --dport 2222 -j ACCEPT`) and
`/var/local/ignite-ssh`, and take the key out of `authorized_keys`.
