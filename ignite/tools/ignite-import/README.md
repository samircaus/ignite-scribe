# ignite-import

Create Ignite notebooks from a small JSON "page spec" (written by hand, a script or an AI
assistant). The tool is another sync **replica**: it posts ordinary, validated operations
(`CreateNotebook`, `CreatePage`, `AddStroke`, `AddText`) to a `scribe-sync-server`, and the Kindle
pulls the notebook through its normal inbound sync. An import only ever **adds** a new notebook; the
separate `delete-*` and `copy-pages` commands below change nothing without `--confirm-write`.

## What you get on the Kindle

| Spec element | Becomes | On the Kindle |
|---|---|---|
| `shapes`, `strokes`, `labels` | native ink strokes | erasable, lasso-movable, undoable |
| `paper` (`lined`/`dot`/`grid`) | page background blob | static (not selectable) |
| `texts` | text object | shown, **not editable on the Kindle** (editable in ignite-web) |

`labels` are drawn with a built-in single-line stroke font (A–Z, 0–9, `.,-:!?'/()+`), so headings
behave like handwriting. Anything the font lacks is an error, not silently dropped.

## Spec

See `examples/poc.json` and the module docs in `src/spec.rs`. Coordinates default to
**page-width units** (`x` 0..1, `y` 0..1.333) so circles stay circles; use
`"coords": "canonical"` for raw 15624 × 20832 units. Points outside the page are rejected, never
clamped.

## Usage

```bash
# 1. Validate and preview, no server and no Kindle involved
cargo run -p ignite-import -- ignite/tools/ignite-import/examples/poc.json --preview-dir /tmp/preview

# 2. Dry run against your server (read-only: fetches the library's render profile)
echo -n "$TOKEN" > /tmp/ignite.token
cargo run -p ignite-import -- spec.json --server http://192.168.1.20:8787 --token-file /tmp/ignite.token

# 3. Create the notebook
cargo run -p ignite-import -- spec.json --server http://192.168.1.20:8787 \
  --token-file /tmp/ignite.token --state-dir ~/.ignite-import --confirm-write
```

- Without `--confirm-write` nothing is ever sent.
- The server must already hold your library, i.e. the Kindle (or web) has synced once. The
  importer copies the render profile (ink pigment/polarity) from an existing page so new pages
  match the device.
- `--state-dir` stores the importer's replica id and next `client_seq`. Keep it between runs.
  Do not point two machines at one library with copies of the same state dir.
- Plain HTTP on a trusted LAN only; the bearer token is sent in a header and never logged.

## Markdown notes

```bash
ignite-import note "My Note.md" [--title T] [--paper lined] [--extra-pages 2] [--spec-out spec.json] ...
```

Lays an Obsidian-flavoured note out as pages with the Kindle's own text metrics
(`src/layout.rs` mirrors `scribe-app`'s `text_paint::wrap_lines`), so nothing runs off a page:
headings, paragraphs, lists, quotes, code, rules as text objects; `- [ ]` / `- [x]` tasks as ink
checkboxes you can tick with the pen. Images/embeds become placeholders. Up to 80 pages.

## Machine-readable use

`--json` prints one JSON object (`ok`, `written`, counts, `warnings`, `error`; after a write also
`notebook.id`, `headServerSeq` and `strokeIds` of the ink the importer drew). `--preview-dir` also
writes `preview.json` (snapshot-shaped pages) plus `paper-<sha>.gray8`, so a caller can render the
plan exactly like a synced page, for example to show a dry run before an assistant or script
writes to the library.

## Listing and deleting

```bash
ignite-import list --server URL --token-file F
ignite-import delete-notebook <index|id> --server URL --token-file F --state-dir D           # shows what would go
ignite-import delete-notebook <index|id> --server URL --token-file F --state-dir D --confirm-write
ignite-import delete-page <index|id> --page 2 --server URL --token-file F --state-dir D --confirm-write
```

These post `DeleteNotebook` / `DeletePage` operations, so every replica converges on the delete
(the Kindle quarantines deleted items). The last page of a notebook cannot be deleted. **A delete
cannot rescue a notebook the Kindle refuses to materialize**: the Kindle applies operations in
order and halts first (see `docs/fork-runbook.md` §9).

## Copying pages between notebooks

```bash
ignite-import copy-pages <src> --pages 1-4 --into <dst> --before 1 --server URL --token-file F --state-dir D --preview-dir P
ignite-import copy-pages <src> --pages 1-4 --into <dst> --before 1 --server URL --token-file F --state-dir D --confirm-write
```

Copies the chosen pages (`1-4`, `2,5`, 1-based) of `<src>` into `<dst>` as new pages, inserted
before page `--before` of `<dst>` (omit it to append). Strokes and typed text are copied verbatim
(same points, pressure and pen style) under fresh ids, so the source is untouched; to *move*
pages, follow with `delete-notebook` or `delete-page` on the source. Rehearse against a scratch
copy of the server's data dir first (`sqlite3 .backup` of the ledger plus `blobs/`).

## Safe area (chrome bands)

`lined`/`dot`/`grid` backgrounds are drawn only between panel rows 140 and 2340. Ignite reserves
the top 130 and bottom 120 rows for its own chrome and **halts inbound sync** on any background
whose bottom band is not uniform paper. Ink and text get the same rule: every stroke point (with
the widest pen radius) and every text box must lie between rows 140 and 2340, i.e. `y` from
0.0774 to 1.2559 in page-width units; anything else is rejected with an error, never clamped.

## Limits and honesty

- Verified: spec parsing, envelope validation, an end-to-end import against a real in-process
  server (library snapshot, blob upload, idempotent resend, existing notebook unchanged).
- **Not verified on a Kindle yet**: how the generated pages render and behave on your device.
  Blank paper is the proof-of-concept path; `lined`/`dot`/`grid` backgrounds are checked only
  host-side.
- Not atomic across batches: if a post fails midway, a partial notebook can exist on the server.
  Delete it from the browser companion or Kindle, then rerun.
- Pen width is not per-stroke; `fixed` strokes use the page profile's radius. `pressure` strokes
  follow the per-point pressure you supply.
