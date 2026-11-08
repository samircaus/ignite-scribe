#!/bin/zsh
# Nightly backup of a scribe-sync-server data dir (run by launchd com.ignite.backup).
#   backup.zsh DATA_DIR BACKUP_ROOT KEEP
# The ledger is copied with SQLite's online .backup (consistent while the server runs). Blobs are
# content-addressed and immutable, so each run hard-links unchanged blobs from the previous
# backup: a nightly copy costs only the new blobs.
set -eu
data=$1 root=$2 keep=${3:-14}
stamp=$(date +%Y%m%d-%H%M%S)
dest="$root/$stamp"
existing=( "$root"/2*(N/) )
existing=( ${existing:#*.partial} )
prev=${existing[-1]:-}
mkdir -p "$dest.partial"
/usr/bin/sqlite3 "$data/ledger.sqlite3" ".backup '$dest.partial/ledger.sqlite3'"
check=$(/usr/bin/sqlite3 "$dest.partial/ledger.sqlite3" "PRAGMA integrity_check;")
[[ $check == ok ]] || { echo "$stamp integrity_check failed: $check" >&2; exit 1; }
if [[ -n $prev && -d $prev/blobs ]]; then
  rsync -a --link-dest="$prev/blobs/" "$data/blobs/" "$dest.partial/blobs/"
else
  rsync -a "$data/blobs/" "$dest.partial/blobs/"
fi
mv "$dest.partial" "$dest"
ops=$(/usr/bin/sqlite3 "$dest/ledger.sqlite3" "SELECT count(*) FROM operations;" 2>/dev/null || echo "?")
echo "$stamp ok operations=$ops size=$(du -sh "$dest" | cut -f1) -> $dest"
# Keep the newest $keep complete backups.
all=( "$root"/2*(N/) )
all=( ${all:#*.partial} )
(( ${#all} > keep )) && for old in "${all[@]:0:$(( ${#all} - keep ))}"; do rm -rf -- "$old"; done
exit 0
