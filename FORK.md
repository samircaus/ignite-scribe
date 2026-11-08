# About this fork

A fork of [Ignite](https://github.com/markrai/ignite) by Mark Rai, developed on a **Kindle Scribe
1st gen (2022) on firmware 5.18.3**. It is based on upstream `3279dca` (0.9.14) and has not yet
been rebased onto later upstream releases. Same license: AGPL-3.0-only (see `LICENSE`, `NOTICE`).

Everything below was used daily on that one device; nothing has been tested on other models or
firmware.

## On the Kindle (`scribe-app`, `kindle-platform`)

- **Pen | Eraser switch** in the notebook toolbar, for pens without an eraser end.
- **Pen linger:** the pen stays armed for 350 ms after lifting, so the next quick stroke no longer
  starts ~0.2 s late (the Pen Up handler went from ~190 ms to 0.23 ms median).
- **No full-screen flash** when the toolbar shows or hides, on erase, undo, page turns or menus;
  only the touched area refreshes. Easier page swipes from near the edge.
- **New toolbar and library:** antialiased Source Sans UI, "Page n of m", page dots, previous/next
  pills, "+ New page"; Notebooks and Books share a 4 × 3 cover grid with folders and rounded menus.
- **Reader (optional, MuPDF):** PDF, EPUB and CBZ from the Kindle's documents folder and Ignite's
  books folder, grouped into My files, Library (Calibre) and one folder per subfolder; reading
  positions are remembered. Build with `--features scribe-app/reader` (see
  `ignite/assets/THIRD_PARTY_NOTICES.md` for MuPDF's licenses).
- **Slightly thicker on-screen ink**, closer to the Kindle's own pen; stored strokes are unchanged.
- **`fb-polarity` override:** if the screen shows white-on-dark (seen on 5.18.3), write `identity`
  or `complement` to `/mnt/us/scribe-app/fb-polarity`. Without the file nothing changes.

## Tools

- **`ignite-import`** (`ignite/tools/ignite-import/`): create notebooks from a JSON page spec, send a
  Markdown note as pages laid out with the Kindle's own text metrics (tasks become ink
  checkboxes), list and delete notebooks or pages, copy pages between notebooks, and dry-run
  previews. It talks to `scribe-sync-server` like any other sync replica.
- **Fast macOS build** (`tools/kindle_macos/build.sh`) for older hard-float firmware, with ELF checks.
- **Wi-Fi deploy** (`tools/kindle_macos/deploy.sh`): backs up the Kindle's state, installs over
  key-only SSH reachable from one Mac, keeps the previous binary. Setup in
  `tools/kindle_macos/README.md`.
- **Sync server as a macOS service** with nightly verified backups (`tools/mac_service/`).

## Docs

- `docs/fork-audit.md`: how the sync protocol, ledger and page model work, and what outside tools
  can safely write.

## Safety

The sync server is plain HTTP with one shared bearer token: use it on a trusted home LAN only.
Wi-Fi SSH opens one port on the Kindle for one address; read the setup and undo steps first.
Back up `/mnt/us/scribe-app/state` before trying any build.
