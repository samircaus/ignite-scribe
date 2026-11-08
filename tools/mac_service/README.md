# Permanent sync server on a Mac

`install.sh` runs `scribe-sync-server` as a launch agent so it survives logouts, crashes and
reboots, and adds a nightly ledger backup.

```bash
tools/mac_service/install.sh --dry-run   # show the plan
tools/mac_service/install.sh             # install/update + restart (stops a hand-started server)
tools/mac_service/install.sh --uninstall # remove both agents (data and backups untouched)
```

| Item | Where |
|---|---|
| Server agent | `~/Library/LaunchAgents/com.ignite.sync-server.plist` (RunAtLoad, KeepAlive) |
| Binary + web bundle (copied, so rebuilds never swap the running server) | `~/Library/Application Support/Ignite/{bin,web}` |
| `ignite-import` (if built) and the Kindle text face, for imports and exact previews | `…/Ignite/bin/ignite-import`, `…/Ignite/fonts/default-sans-v1.ttf` |
| Data (ledger + blobs), token verifier | `$IGNITE_HOME/server-data`, `$IGNITE_HOME/token.verifier` (default `IGNITE_HOME=~/.ignite`; not moved) |
| Backup agent | `com.ignite.backup`, daily 03:20, keeps 14 |
| Backups | `$IGNITE_HOME/backups/nightly/<stamp>/` (SQLite online `.backup` + integrity check; blobs hard-linked to the previous backup) |
| Logs | `~/Library/Logs/ignite-sync-server.log`, `~/Library/Logs/ignite-backup.log` |

After a new `cargo build --release -p scribe-sync-server` (or `-p ignite-import`), rerun
`install.sh` to deploy it.

Each path can also be set on its own: `IGNITE_DATA_DIR`, `IGNITE_TOKEN_VERIFIER`,
`IGNITE_BACKUP_DIR`, `IGNITE_BIND` (default `0.0.0.0:8787`), `IGNITE_BACKUP_KEEP`.

Create the data dir and token once:

```bash
mkdir -p ~/.ignite/server-data
./target/release/scribe-sync-server new-token --verifier-out ~/.ignite/token.verifier > ~/.ignite/token.txt
chmod 600 ~/.ignite/token.txt
```

The Kindle stores the server URL in `/mnt/us/scribe-app/state/sync/config.json`, so give the Mac a
fixed address: a DHCP reservation in the router. On Wi-Fi, macOS uses a private MAC address per
network; set System Settings → Wi-Fi → (i) → Private Wi-Fi address to **Fixed** first, then reserve
the address for that MAC.

Plain HTTP with one shared bearer token: trusted home LAN only, never the open Internet.
