# Ignite fork: technical audit and opportunity assessment

> Written before the fork's changes were made, as an assessment for the fork's owner ("you" below).
> Kept because sections 2 and 4–6 describe how Ignite's sync protocol, ledger and page model work,
> and whether outside tools can write valid notebooks. What the fork has built since is in
> [`../FORK.md`](../FORK.md).

Audit date: 2026-11-04. Base: `main` @ `3279dca` (Ignite 0.9.14), plus the local, uncommitted
`tools/kindle_macos/` build path. Target device: Kindle Scribe 1st gen (2022), firmware 5.18.3.

**Evidence labels.** *[V]* = read in source/tests or run on this Mac. *[D]* = stated in docs/web
only. *[?]* = not verified; needs the device or a source I could not reach. Nothing in this
document was exercised on a Kindle.

---

## 1. Executive summary

**Ignite is a good foundation for what you want, mainly because of its data model and sync API,
not its on-device UI.**

What makes it good:

- Ink is stored as **vector strokes** (f32 x/y + optional pressure) in a fixed 15624×20832 page
  space, as append-only JSONL. *[V]* `notebook-core/src/point.rs`, `sync-protocol/src/geometry.rs`,
  `scribe-app/src/store.rs`.
- There is already a **documented, versioned, validated interchange format**: `SyncSnapshotV1` and
  `SyncOperationV1` (create notebook/page, add stroke, add text, move page, page links, …) over an
  HTTP/JSON `/v1` API. *[V]* `sync-protocol/src/ops.rs`, `snapshot.rs`, `scribe-sync-server/src/http.rs`.
- The **browser companion is already a second writer** on that API, so "something other than the
  Kindle creates content, the Kindle pulls it" is an existing, tested code path, not a new one.
  *[V]* `web/src/replica/*`, `s2c_inbound_tests.rs`.
- Native Amazon notebook import already exists (one-way, read-only). *[V]* `nbk_import.rs`, `nbk-reader`.
- Code quality around durability is unusually high (crash-ordered writes, fail-closed validation,
  ~95k lines including a very large test suite).

What is weak for your goals:

- **Writing-experience features are thin**: no zoom, no redo, no search (home search bar is an
  inert placeholder), lasso can only *move*, no templates/paper types, no text authoring on the
  Kindle. *[V]* greps below.
- **Backgrounds are 4.6 MB raw gray8 bitmaps per distinct image**, not a template spec. *[V]*
- **Text objects are display-only on the Kindle** and use one bundled sans font. *[V]*
- The project is a sprawling 17-crate workspace with a large "recovery" subsystem you will never
  use. It is maintainable only if you treat most of it as frozen.
- **Compatibility with 5.18.3 is unverified upstream** (verified firmware is 5.18.6). Your Mac build
  is plausibly *more* conservative than upstream's (see §3), but that is untested on hardware.

**Recommendation.** Keep Ignite. Do **not** start with MCP, Cloudflare or handwriting recognition.
Start with a small Rust **import tool** that turns a simple page-spec JSON into `SyncOperationV1`
envelopes and posts them to your local `scribe-sync-server`. That one tool unlocks AI-generated
journals, mind maps, SVG-derived ink and (later) MCP, with zero Kindle-side changes for the first
milestone. Then spend Kindle-side effort on the few daily-writing gaps (redo, zoom, paper types).

---

## 2. Architecture and feature audit

### 2.1 Architecture

```
 Mac / AI assistant                LAN (plain HTTP, bearer token)             Kindle Scribe (5.18.x)
 ┌──────────────────┐   POST /v1/libraries/{id}/operations   ┌──────────────────────┐
 │ importer (NEW)   │ ─────────────────────────────────────▶ │ scribe-sync-server   │
 │ spec.json/SVG →  │   PUT  /v1/blobs/{sha256}              │  SQLite ledger,      │
 │ SyncOperationV1  │                                        │  canonical ordering  │
 └──────────────────┘                                        └─────────┬────────────┘
 ┌──────────────────┐   same API, via WASM encoder                     │ GET …/operations?after=N
 │ ignite-web (PWA) │ ◀────────────────────────────────────────────────┤ GET /v1/blobs/{sha}
 │ React+Canvas2D,  │                                                  ▼
 │ IndexedDB outbox │                                        ┌──────────────────────┐
 └──────────────────┘                                        │ scribe-app (static)  │
                                                             │  sync_inbound → inbox│
        launched by Library entry                            │  → sync_materialize  │
 appmgrd ─▶ scribe-lifecycle-host.sh ─▶ scribe-lifecycle ───▶│  → store.rs journals │
                          (LIPC, glibc, dynamic)             │  ink/raster → fbdev  │
                                                             └──────────────────────┘
  Disk on Kindle: /mnt/us/scribe-data/<notebook-N>/<page-N>/{page.meta, background.raw,
                  strokes.jsonl, edits.jsonl, texts.jsonl, links.jsonl}   notebook.meta
```

Crates (all *[V]* from `Cargo.toml` and sources):

| Layer | Crates | Role |
|---|---|---|
| Geometry/model | `notebook-core` | Point, Stroke, canonical↔panel coords, hit-test, eraser clipping |
| Interchange | `sync-protocol`, `sync-engine` | Snapshot/op types, validation, canonical JSON + SHA-256, reducer |
| Kindle runtime | `scribe-app`, `kindle-platform`, `scribe-lifecycle`, `ink-probe` | UI, ink, evdev/fbdev/e-ink ioctls, LIPC lifecycle, hardware lab tool |
| Servers/tools | `scribe-sync-server`, `scribe-server`, `scribe-sync-tool`, `backup-*`, `portable-backup` | Ledger, pairing/control, export/restore |
| Import | `nbk-reader` | Parses Amazon NBK (KDF/SQLite + Ion) in-process |
| Web | `ignite/web`, `scribe-web-wasm` | Browser replica; Rust WASM does all wire encoding |

### 2.2 The ten questions

| # | Question | Finding |
|---|---|---|
| 1 | Rendering / pen / pressure / storage | Pen samples → `StrokeBuilder` (`notebook-core/src/stroke.rs`) → canonical points → software raster of **square-brush lines with integer radius 1–4 px** straight into the framebuffer (`scribe-app/src/render.rs`, `kindle-platform/src/raster.rs`). Pressure maps through a sealed 4-band curve (`pressure_curve.rs`, q thresholds 820/2866/3685 of 4095). Strokes persist as JSONL, `PersistedStrokeV1/V2` (`store.rs:330-500`). *[V]* |
| 2 | Data model | Library → notebooks (`notebook.meta`: ordered `page_ids`) → pages (dir with `page.meta`). Sync identity (UUIDs) is bound separately in `identity.json` (`sync_local.rs`). Links and texts are per-page JSONL. Folders exist. *[V]* |
| 3 | Create/edit/save/load/export/delete | Create/delete/reorder via `library.rs`/`notebook.rs` with fsync-ordered, quarantine-based deletes (`.page-trash`). Export: `backup-export` tool, Linux server export; web viewer. No PDF/PNG export. *[V]* |
| 4 | Vector or bitmap | **Strokes: vector**, replayed onto the page background at open. **Background: bitmap** `background.raw` (gray8, 1860×2480 panel pixels). *[V]* |
| 5 | Can external apps create valid notebooks | **Yes, via the sync ledger**: any client holding the bearer token can POST `CreateNotebook`/`CreatePage`/`AddStroke`/`AddText`. The server validates strictly (400/409/422) so invalid content is refused rather than corrupting a Kindle. Direct file-writing into `/mnt/us/scribe-data` is possible but undocumented and bypasses sync identity; do not do it. *[V]* |
| 6 | Web companion ↔ Kindle | Both talk to `scribe-sync-server` (`/v1`) and never to each other. Kindle polls the ledger tail (`sync_inbound::pull_once`), stages to an inbox, applies at "quiet" moments only (`input_lag::inbound_may_run`: pen not in range, no queued input, 1.5 s idle). *[V]* |
| 7 | Sync / conflicts | Implemented, not just planned. Server assigns total order; reducer applies in canonical order (`sync-engine/src/apply.rs`); erase applies only to lineages the author *observed*; page-move/create use identity-based "after X, else end"; text is whole-object last-writer-wins. Large test suite (`s1`, `s2b`, `s2c`, `sync_*_tests`). Plain HTTP, single shared bearer token, LAN only. *[V]* |
| 8 | Works on 5.18.3 + PaperTiger | **Unknown.** Verified: 5.18.6 only (`docs/compatibility.md`). Web search found no source for "PaperTiger" and I could not verify it. See §3. *[?]* |
| 9 | vs Grimoire / native | §4. |
| 10 | Build/deploy/launch/debug | §3.3. |

### 2.3 Feature matrix (implemented vs partial vs absent)

| Feature | Status | Evidence |
|---|---|---|
| Pen ink, pressure-width | Implemented | `render.rs`, `pressure_curve.rs` |
| Palm rejection | Not found as a feature; touch is a separate input path (`touch.rs`, `touch_mt.rs`), pen hover gates sync apply. Whether touch is suppressed while the pen is in range **[?]** | `kindle-platform/src/touch_mt.rs` |
| Stroke eraser (partial erase, fragments) | Implemented | `eraser.rs`, `clip.rs` |
| Lasso **select + move** (finger-drawn) | Implemented | `lasso.rs` |
| Lasso scale / copy / paste / duplicate | **Absent** | no matches in `lasso.rs` |
| Undo (newest stroke; also last move) | Implemented, minimal | `runtime.rs:2103 undo_and_present` |
| Redo | **Absent** (`redo_available=false` is logged) | `runtime.rs:2122` |
| Zoom / pan | **Absent** (no `zoom`/`pinch` anywhere under `ignite/kindle`) | grep |
| Page insert / delete / reorder / navigate | Implemented | `notebook.rs`, `nav.rs` |
| Notebook folders, rename, reorder | Implemented | `library.rs`, `folders.rs` |
| Library search | **Placeholder** ("search is not implemented yet") | `home.rs:270` |
| Page links (tap a point → other page) | Implemented | `link_picker.rs`, `link_paint.rs`, `SyncPageLink` |
| Text objects | **Display-only on Kindle**; authored in web (`web/src/authoring/text*.ts`). Kindle keyboard is for titles only | `keyboard.rs`, `runtime.rs:153`, `store.rs:1` |
| Paper types / templates | **Absent**. New pages are white; background can only be a raw bitmap blob | `create_new_page`, `BackgroundRef` |
| Autosave / crash recovery | Strong: append-only journals, truncated-tail handling, fsync ordering, migrations tested | `store.rs`, `page_ledger.rs` |
| Native NBK import (one-way) | Implemented, with receipts and crash-safe state machine | `nbk_import.rs` |
| Optional LAN sync + PWA | Implemented | above |
| Handwriting recognition / OCR | Absent; page links doc explicitly says "no OCR" | `ops.rs` header |
| PDF/PNG/SVG export | Absent | none in tree |
| Battery / memory metrics | Instrumentation exists for latency stalls and open-time; no battery data **[?]** | `input_lag.rs`, `examples/open_bench.rs` |

Docs referenced from source but missing from this repo: `docs/SYNC_ARCHITECTURE.md`,
`docs/NBK_IMPORT.md`, `docs/KINDLE_PROBE.md`. The upstream README also calls performance
"Review 79 … open". Treat latency as unproven until you measure on your own device.

---

## 3. Compatibility with your exact setup

### 3.1 Firmware 5.18.3

- Upstream "Verified" is 1st-gen Scribe on **5.18.6** (`README.md:67`); several kernel/evdev/fb
  assumptions are written as "confirmed on FW 5.18.6" in `kindle-platform`. 5.18.3 is three point
  releases older. Same major generation, so the input/display ABI is probably the same, but
  nothing in the repo asserts it. *[?]*
- **Binary linkage is the likeliest 5.18.3 risk, and your fork already mitigates it.** Upstream
  builds `scribe-app` as a *dynamic* glibc binary on Debian bullseye (glibc 2.31 headers;
  `tools/docker_build_armv7hf.sh`, `tools/pinned-docker-images.txt`). Your Mac path builds
  `scribe-app` as **static musl** (no glibc dependency) and only the small LIPC host
  `scribe-lifecycle` against an EGLIBC 2.17 sysroot. `tools/kindle_macos/verify_elf.sh` enforces
  that. I ran it against your existing `target/kindle-macos/release/`: **OK, lifecycle needs at most
  GLIBC_2.16 (ceiling 2.18)**. *[V]* That the device's actual glibc is ≤2.18 is your assumption in
  `PINNED.txt`; I did not check the device. *[?]*
- **Caveats on the Mac build:** it is a dev path, not release-equivalent; static musl changes
  `getaddrinfo`/DNS behavior (relevant to sync hostname resolution, so prefer an IP or test mDNS);
  default rustc here is 1.88 but the lockfile needs 1.89 (the build script pins 1.89.0, plain
  `cargo test` does not).
- **PaperTiger:** I could not find it via search. Unverified; tell me what it provides (SH
  launcher? root helper? kernel-level?). Ignite needs a Library scriptlet launcher, a root helper
  and appmgr registration (`README.md:41`, `tools/kindle_app/registration/ignite.json`).
- Pre-flight: `Diagnose-Ignite.sh` (installer helper) exists to collect evidence; run it first.

### 3.2 Display

First-gen Scribe panel is 1860×2480 grayscale; Ignite expects `grayscale=2` (inverted) polarity
there (`fb_polarity.rs`). A 4.6 MB background per page is written whole (no compression) — fine for
a handful of templates because blobs are content-addressed and shared, but it is storage-heavy for
per-page unique images.

### 3.3 Build, deploy, launch, debug

| Step | How | Status |
|---|---|---|
| Host tests | `cargo +1.89.0 test -p notebook-core -p sync-protocol -p sync-engine -p nbk-reader --locked` | **Ran: all pass** *[V]* |
| Full lib suite | `cargo +1.89.0 test -p scribe-app --lib --locked` | Not run (large) |
| Fast ARM build | `bash tools/kindle_macos/build.sh` (needs zig 0.17, cargo-zigbuild 0.24, colima/docker for lifecycle) | Your artifacts exist; ELF check passes *[V]* |
| Official package | `bash scripts/kindle/package-release.sh` → `Install Ignite vX.sh` | Docker, digest-pinned |
| Install | copy single `.sh` into `Documents/`, tap it on the Kindle | *[D]* |
| Quick iteration | `ink-probe` and `scribe-app` outputs; how to swap them into an installed device without re-running the installer is **not documented** — this is gap #1 for fork velocity | *[?]* |
| Debug | `Diagnose-Ignite.sh`, device logs; `scribe-sync-tool snapshot/compare/validate-snapshot` for data-level checks | *[D]/[V]* |

---

## 4. Comparison: Ignite vs Grimoire vs native notebooks

Grimoire facts come from the repo pages (`ganten1998/grimoire-scribe`: README, FORMAT.md,
DESIGN.md, KINDLE.md). Parts of those came back as summaries, so treat fine detail as *[D]*.
Native-Kindle claims are from this repo's `nbk-reader` and from general knowledge, not Amazon docs.

| Dimension | Ignite | Grimoire | Amazon native |
|---|---|---|---|
| Runtime | Rust, static/dynamic ARM, own UI | C core + some Rust; native, FBInk output | Closed |
| Stroke storage | JSONL vector, f32 canvas coords | Binary `QPG1` pages: point lists with pressure, tilt, time, replayed | KDF/SQLite + Ion (read by `nbk-reader`) |
| Pen quality | Square brush, radius 1–4, pressure-banded; no tilt | **Nine pens**, tunable pressure/tilt, calligraphy nib | Best-tuned, firmware-integrated |
| Templates | None | **Many** (rules, dot, graph, Cornell, music staves, tab, custom PNG/JPG) | Fixed set |
| Import | NBK→native; sync API (programmatic) | PNG/JPG pages, photos as paper; PDF only via KOReader | PDF markup, Send-to-Kindle |
| Export | Backup archive; no PDF/PNG | **PDF/PNG/zip over Wi-Fi/USB, SSH pull** | Cloud/email/export |
| Undo/redo | Undo only | Undo + redo, per page | Yes |
| Lasso | Select + move | Not stated | Yes |
| Page links | **Yes** | **Yes** (plus graph view) | Limited |
| Sync | **Ledger + browser replica**, conflict rules | None (backup routes only) | Amazon cloud |
| Programmatic API | **Yes (`/v1`)** | None stated | None |
| Firmware | 5.18.6 verified | 1st gen "5.16.3 or newer works", tested daily on 5.20.1 (**5.18.x not named**) | — |
| License | **AGPL-3.0-only** | GPL-3.0(-or-later) | Proprietary |
| Maturity signal | Hardware-gated releases, deep tests | 761 commits, beta, fast-moving | Production |

**Worth borrowing conceptually (clean-room, no code copy):**

1. Paper as a **small numeric/enum value + parametric renderer** rather than a bitmap — exactly
   your "template spec" idea, and it is what Ignite lacks.
2. Redo and "keep damaged files instead of overwriting".
3. Whole-notebook **PDF export**.
4. Tilt/pen-type tables are *not* worth it for you: the first-gen digitizer may not report tilt
   (Grimoire itself says tilt does nothing if the pen reports none).

**Licensing.** Ignite is AGPL-3.0-only; Grimoire is GPL-3.0. Reading either for ideas is fine.
Copying Grimoire code into an AGPL-only tree is a licensing problem (GPL-3.0-only isn't
AGPL-compatible without "or later"; its README is mixed). Don't copy code. Your fork is also
AGPL, so a hosted backend you run for yourself triggers no obligation, but offering it to others
would require publishing that backend source.

---

## 5. External import feasibility matrix

Your four capability classes: **A** static background, **B** movable object, **C** native
editable strokes, **D** editable text.

What Ignite can represent today *[V]*:

- **C**: `SyncStroke` (polyline, optional per-point pressure). Erasable (partial), lasso-movable,
  undoable, synced. This is the only fully "native" content type on the Kindle.
- **A**: `BackgroundRef::Blob` gray8 bitmap, immutable, not selectable/movable/erasable. Must be
  exactly the panel size (1860×2480; stride accepted: width or line_length) — `sync_materialize.rs:129-160`.
- **D**: `SyncText` (default-sans-v1 only, ≤8192 chars). Editable in the browser; **display-only
  on Kindle**. No lasso selection of text.
- **B**: **does not exist.** There is no movable image/object type. An "image object" would need a
  new op + renderer.

| Source | Best target | Feasibility | Transformation | Complexity | Limits |
|---|---|---|---|---|---|
| SVG paths/lines | **C** | **High** | Flatten Béziers/arcs to polylines, apply SVG transform, map viewBox→15624×20832, drop fills, cap at 100 000 pts/stroke | S–M | Fills, gradients, text, clip paths dropped. Stroke width is not stored; width = fixed radius or pressure curve, so line weight isn't controllable per-stroke. Use `legacy-fixed-v1` for uniform diagram lines |
| "Handwritten" headings from fonts | **C** (not D) | **High** | Single-stroke fonts (Hershey) → polylines directly; outline fonts → centerline needs extra work or accept double-line outlines | S (Hershey) / L (outline) | A TTF as `SyncText` is not possible (only `default-sans-v1`). Hershey looks like pen lettering and edits like handwriting |
| PNG/JPEG | **A** | Medium | Resize/dither to 1860×2480 gray8, PUT blob, reference in `CreatePage` | S–M | Not movable; 4.6 MB each; E-ink gray shading vs ink contrast. **B** requires new feature (L) |
| Illustration → strokes | **C** | Medium–Low | Potrace/centerline tracing → polylines | M | Quality varies; many points; cheaper to do as **A** |
| PDF page | **A** (rasterise) or **C** (vector paths) | **A: High; C: Medium** | Rasterise with pdfium/mupdf to 1860×2480 gray8 | S (A) | Text is not selectable. Vector extraction possible for line art |
| Markdown | **C** headings + **A** body, or **D** | Medium | Typeset to bitmap (A) for reading text; headings as strokes; or `SyncText` if you accept display-only | M | `SyncText` wraps with Ignite's own metrics (`text_paint.rs`); long notes need pagination |
| Diagrams / mind maps / flowcharts | **C** | **High** | Generate geometry directly (boxes, arrows, curved branches, Hershey labels) | S–M | Best first use case: it is just polylines |
| Music staves | **A** (staff template) | **High** | Procedural gray8 render of staves | S | One 4.6 MB blob per distinct layout, shared across pages |
| Amazon notebooks | **C** | **Done** (strokes only) | `nbk_import.rs` | — | One-way, once; pending actions refused |

**Key conclusion:** with zero Kindle changes you can already deliver **A + C** (backgrounds you
render on the Mac, plus real editable ink). **D** works only if display-only is acceptable. **B**
needs real new code on both Kindle and protocol.

---

## 6. Templates

`BackgroundRef` is today either `Blank` or a gray8 blob — a "template" is just a precomputed
bitmap. Options:

| Option | Offline on Kindle | Size | New code | Verdict |
|---|---|---|---|---|
| **Prerendered blob backgrounds (existing)** | Yes | 4.6 MB each, deduped by SHA | None Kindle-side | **Use now** |
| **Parametric `PaperSpec` rendered on the Kindle** | Yes | ~50 bytes in `page.meta` | New `BackgroundRef::Paper` variant (protocol bump, old decoders fail closed), renderer in `scribe-app`, same in WASM/web | **Right long-term design**, M–L |
| Template as reusable *page definition* (bg + guide strokes + text) | Yes | Small | Mostly tooling, not Kindle | Good fit for the generator |

Recommendation: ship templates as **blobs rendered by a Mac tool** first (lined, dot, grid, music,
Cornell, daily, commonplace, reading-notes, mind-map frame). Because identical blobs are
content-addressed they are stored once. Only introduce `Paper` specs once you know which 6–8
templates you really use. Note: a page's background cannot be changed after creation (immutable
by design; `store.rs` module docs), so "switch paper" means new page + move strokes, not an edit.

A simple external spec (§8) should describe a **page** = `{paper, strokes[], texts[], links[]}`,
with `paper` = a named built-in generator (`"lined"`, `"dot-5mm"`, `"staff"`, `"png:path"`).

---

## 7. AI-native workflow and architecture

### 7.1 Can the six workflows work?

| Workflow | Needs | Feasible with current Kindle code? |
|---|---|---|
| 1. 20-page journal with prompts, sketches, blank space | Importer: `CreateNotebook` + `CreatePage`×19 + backgrounds + strokes/text | **Yes** (prompts as Hershey strokes, sketches as polylines) |
| 2. Mind map to annotate | Same | **Yes** |
| 3. Reading notes → notebook | Same; text as bitmap or strokes | **Yes** (A or C) |
| 4. Music discovery notebook | Same; staff/list templates | **Yes** |
| 5. Export handwriting for transcription | Read side: `GET /v1/libraries/{id}/snapshot` gives all strokes; rasterise per page; send to a vision-capable model | **Yes**: no on-device recognition needed |
| 6. Add a page without overwriting handwriting | `CreatePage` with `after_page_id`; append-only ledger + identity-based ordering means existing strokes are never touched | **Yes**: this is exactly what the protocol is designed for |

### 7.2 Recommendation: local first, Cloudflare later (or never)

- You already need `scribe-sync-server` on the LAN for any sync. It *is* the local-first backend.
- **MCP**: a thin MCP server (stdio, local) that wraps the importer + snapshot read gives
  Claude/ChatGPT desktop the tools `create_notebook(spec)`, `append_page(notebook, spec)`,
  `get_notebook(id)->PNGs/JSON`. It needs no cloud. Build only after the importer exists.
- **Cloudflare Worker + D1 + R2**: the sync protocol, blobs and ledger would map onto it, but it
  means re-implementing `scribe-sync-server` (Rust+SQLite) in TS, the Kindle talks plain HTTP to
  a LAN host today, and TLS from a 5.18 Kindle needs a CA bundle check *[?]*. It buys remote
  access only. Challenge: use **Cloudflare Tunnel/Access in front of the same LAN server** if you
  want off-network access, instead of a rewrite.
- Auth: today one shared bearer token, whole-library scope. Fine on a home LAN; for an
  AI-facing endpoint add a second read-only token (small change in `auth.rs`).
- Handwriting recognition: do it **off-device** from the exported snapshot (vision model on PNGs).
  Avoid on-device models entirely.

---

## 8. Proposed architecture

Keep the existing three tiers; add one tool and keep everything else frozen.

```
 generators (Python/AI/SVG) ──▶ page-spec.json ──▶ ignite-import (NEW Rust CLI, uses sync-protocol)
                                                        │  validated SyncOperationV1 + blobs
                                                        ▼
                                            scribe-sync-server (existing, LAN)
                                                        ▲        ▲
                                          Kindle scribe-app     ignite-web
 optional later: MCP server ─▶ ignite-import + snapshot reader;   Cloudflare Tunnel (not rewrite)
```

Principles: reuse `sync-protocol` so output is canonical by construction (hand-writing canonical
JSON in Python is how you get HTTP 400s); no new Kindle code for milestone 1; recovery subsystem
stays disabled; nothing in this plan alters an existing notebook (creates are additive; the
protocol has no overwrite).

**Page-spec sketch** (normalised 0..1 coordinates so generators are device-independent):

```json
{ "notebook": {"title": "Creative journal"},
  "pages": [{
    "paper": {"kind": "lined", "spacing_mm": 7},
    "strokes": [{"style": "fixed", "points": [[0.10,0.08],[0.30,0.08]]}],
    "texts": [],
    "links": []
  }] }
```

---

## 9. Prioritised roadmap

Effort = focused solo days, rough. Not benchmarked against this codebase's review/gate process,
which is heavy (hardware qualification gates); multiply device-facing work by ~2–3.

| # | Improvement | User benefit | Status today | Approach | Size | Effort | Risks / deps | Acceptance |
|---|---|---|---|---|---|---|---|---|
| 1 | **Fast install-to-device loop** | Iterate without full installer | Undocumented | Script to stop app, swap `scribe-app`/`scribe-lifecycle`, back up first; reuse `Diagnose`/`Backup` helpers | S | 1–2 d | Needs root helper path knowledge | One command from Mac → relaunched app, data intact |
| 2 | **Importer CLI** (`ignite-import`) | Everything in §7 | Missing | New crate on `sync-protocol`; spec→ops; blob PUT; dry-run + `validate-snapshot` | M | 3–5 d | Needs genesis/library id handshake with server | Spec with heading+diagram+blank page appears on web and Kindle; `compare` shows equal |
| 3 | **SVG→strokes front end** | AI/Mac drawings as ink | Missing | Python or Rust; flatten paths, Hershey text | S–M | 2–3 d | Point density vs raster cost | Imported shapes erasable/lasso-movable on device |
| 4 | **Template pack** (6–8 blobs) | Journals, grids, music | Missing | Mac renderer → gray8 1860×2480 | S | 2 d | 4.6 MB each; polarity | Pages render with correct polarity, once-stored by SHA |
| 5 | **Redo** | Everyday editing | Absent | Mirror undo stack; re-add via existing ops | S–M | 2–4 d | Interacts with sync lineage rules | Undo×3, Redo×3 restores identical strokes, survives sync |
| 6 | **Zoom/pan** | Detail work, sketching | Absent | Viewport transform exists in `notebook-core/transform.rs`; UI + redraw on e-ink | L | 8–15 d | **Highest latency risk**; e-ink refresh | No stall >50 ms in `input_lag` stats while drawing |
| 7 | **Export snapshot → PNG/PDF** (Mac tool) | Hand to AI, archive | Missing | Rasterise strokes+bg from snapshot | M | 3 d | Matches Kindle raster only approximately | PDF of notebook with all pages |
| 8 | **MCP server** | Claude/ChatGPT drive notebooks | Missing | Wrap 2 + 7 | M | 3–4 d | Auth scoping | "Add a page" tool leaves existing strokes byte-identical |
| 9 | Lasso copy/scale | Reuse sketches | Absent | New ops (`AddStroke` of transformed copies) | M | 5–8 d | UI on e-ink | Duplicate yields new IDs, synced |
| 10 | `BackgroundRef::Paper` parametric | Light templates | Absent | Protocol variant + renderer ×3 platforms | L | 10+ d | Protocol/versions | Old clients fail closed, new render equal across web/Kindle |
| 11 | Library search | Find notebooks | Placeholder | Title filter only (no OCR) | S | 2 d | Keyboard exists but is title-only | Typing filters tiles |
| 12 | Movable image objects (class B) | Real collage | Absent | New op + renderer | L | 15+ d | Protocol bump | Select/move image like strokes |

**Quick wins:** 1, 2, 3, 4 (nothing in Kindle code), then 5.
**Defer:** Cloudflare backend, OCR, tilt/pen types, text editing on Kindle.

---

## 10. Proof of concept (proposed, not yet built)

Your 8-step experiment is feasible. Smallest version:

1. `ignite-import` reads a page-spec: Hershey heading, a few boxes+arrows as polylines, lined
   background blob (or blank), leave the lower half empty.
2. Posts `CreateNotebook` (new UUIDs, title "PoC") to a **local** `scribe-sync-server` that the
   Kindle is paired with. Purely additive: no existing notebook is touched.
3. Verify on the Mac first via `ignite-web` and `scribe-sync-tool snapshot`/`compare`.
4. On the Kindle: confirm render; try erase, lasso-move, undo on the generated strokes (expected:
   all work, since they are ordinary `AddStroke`s); write by hand; close/reopen; export snapshot
   back to the Mac.

Expected results: strokes selectable/movable/erasable (C); background not (A); text not editable
on device (D). It needs a one-time server + pairing setup (`docs/self-hosting.md`).

### 10.1 PoC result (2026-11-04, Kindle Scribe 1st gen, firmware 5.18.3)

Done with `ignite/tools/ignite-import` (branch `feat/ignite-import`), a local
`scribe-sync-server` on the Mac, and `examples/poc.json`:

| Step | Result |
|---|---|
| Official 0.9.14 installer binaries | **Cannot run**: need GLIBC_2.30, device has glibc 2.20 |
| Fork build (static musl app + glibc 2.16 lifecycle) via `package_release.py` | Installs and launches; home screen in ~1.6 s, ~25 MB RSS |
| Display polarity | Device reports `grayscale=1` but displays complemented bytes inverted. Fixed with optional `/mnt/us/scribe-app/fb-polarity` = `identity` (branch `fix/fb-polarity-override`) |
| Kindle → server upload of existing library | 216 ops acknowledged |
| Import of generated notebook (47 envelopes, 46 strokes) | Server accepted; Kindle applied (`notebooks_created=1`, `strokes_appended=46`) in ~0.7 s; user confirmed it renders correctly |
| Handwriting added on the Kindle | Synced back to the server (page grew from 46 to 59 strokes) |
| Export back to Mac | `export/snapshot.json` (lossless) + SVG render shows generated and handwritten strokes together |

Not yet verified: erase/lasso/undo on generated strokes, close/reopen persistence (observed
only indirectly via sync), paper backgrounds (`lined`/`dot`/`grid`), text objects, battery.

Gotchas found: the Kindle app does not re-read `config.json` while running (restart it); the
Mac's Wi-Fi address changed (use a fixed address); the Kindle's `/mnt/us` is FAT-style, so
Dropbear ignores `authorized_keys` there (use an SSH ControlMaster socket instead).

---

## 11. Concrete implementation plan (references)

- New crate `ignite/tools/ignite-import` (add to workspace `members`), depending on
  `sync-protocol`; model after `scribe-sync-tool/src/lib.rs` (`DevReplica`, HTTP helpers around
  line 230–330) which already builds and posts valid envelopes.
- IDs/envelope: `sync-protocol/src/ids.rs`, `ops.rs` (`SyncEnvelopeV1`, `PageSpec`), render profile
  from `scribe-web-wasm::sealed_create_notebook_render()` (`lib.rs:82`) — **copy these exact
  values** so pressure/raster profile matches the Kindle's.
- Stroke validation limits: `content.rs` (`MAX_STROKE_POINTS = 100000`), points within
  0..15624 × 0..20832, pressure 0..1.
- Background blob: gray8 1860×2480, `BackgroundRef::Blob{sha256,byte_len,format}`; `PUT
  /v1/blobs/{sha}` before posting ops (server returns 422 `missing_blobs` otherwise).
- Batches ≤100 envelopes, JSON body ≤4 MB (`http.rs`).
- Tests: follow `scribe-sync-server/tests/api.rs`; fixtures under `web/tests/fixtures`.

## 12. Open questions and risks

1. **PaperTiger**: what is it, and does it provide the root helper and LIPC that Ignite's
   installer checks? *[?]*
2. Does the 5.18.3 glibc really have ≤2.18? (`ldd --version` over SSH.) *[?]*
3. Is palm rejection actually implemented, or relied on from firmware? *[?]*
4. Real pen latency and battery cost of the software raster on your unit. Upstream notes an open
   performance review. *[?]*
5. How does the Kindle behave on first pairing with a server on your LAN (mDNS `_scribe._tcp`
   vs fixed IP) with a musl-static build? *[?]*
6. Which exact fork/branch is "the quicker build" — it is currently untracked/uncommitted.
7. Plain-HTTP, single-token auth: do not put this on the open Internet.
8. Hand-edited `scribe-data` files bypass sync identity — avoid.
9. AGPL obligations if you ever host a modified server for others.
10. Docs referenced but absent in this tree (`SYNC_ARCHITECTURE.md`, `NBK_IMPORT.md`,
    `KINDLE_PROBE.md`): the sync design intent is only in code comments.

---

### Sources consulted (external)

- Grimoire repo and docs: <https://github.com/ganten1998/grimoire-scribe>
- Jailbreak context for 5.18.3 (Sanctuary range): MobileRead / KindleModding as returned by search;
  I did not independently confirm.
