//! `ignite-import` CLI. See the crate docs and `ignite/tools/ignite-import/README.md`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ignite_import::build::{build, default_render, Ctx, Plan};
use ignite_import::client::{discover_library, fetch_target, submit, Http, ReplicaFile, Transport};
use ignite_import::note::{from_markdown, NoteOptions};
use ignite_import::preview::page_svg;
use ignite_import::spec::{Paper, Spec};
use serde_json::{json, Value};
use sync_protocol::{
    LibraryId, ReplicaId, SyncOperationV1, CANONICAL_PAGE_HEIGHT, CANONICAL_PAGE_WIDTH,
};
use uuid::Uuid;

const USAGE: &str = "\
ignite-import <spec.json> [options]
ignite-import note <note.md> [--title T] [--paper blank|lined|dot|grid] [--extra-pages N] [options]

  Validates a page spec (or lays out a Markdown note with the Kindle's own text metrics) and,
  with --confirm-write, creates it as a NEW notebook on an Ignite sync server. Existing
  notebooks are never modified.

ignite-import list --server URL --token-file FILE
ignite-import delete-notebook <index|id> --server URL --token-file FILE --state-dir DIR [--confirm-write]
ignite-import delete-page <index|id> --page N --server URL --token-file FILE --state-dir DIR [--confirm-write]
ignite-import copy-pages <index|id> --pages 1-4 --into <index|id> [--before N] --server URL
              --token-file FILE --state-dir DIR [--preview-dir DIR] [--confirm-write]

  copy-pages copies pages verbatim (every stroke point and pressure, typed text) into another
  notebook, before its page N (default: at the end). The source notebook is not changed.

  list shows notebooks with their indexes. The delete commands only describe what they would
  delete unless --confirm-write is given. Deleted items stay quarantined on the Kindle.

Options:
  --preview-dir DIR    write page-NN.svg previews and preview.json (snapshot-shaped pages plus
                       paper-<sha>.gray8 backgrounds) for an exact render; no device needed
  --spec-out FILE      (note) also write the generated page spec
  --server URL         http://host:port of scribe-sync-server (read-only without --confirm-write)
  --token-file FILE    file holding the bearer token (or set IGNITE_TOKEN)
  --library ID         library id (default: the server's only library)
  --state-dir DIR      where the importer keeps its replica id / client_seq (needed to write)
  --confirm-write      actually post to the server (otherwise: validate and report only)
  --json               print one JSON object (ok, written, counts, warnings, error) instead of text
";

fn opt(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn token(args: &[String]) -> Result<Option<String>, String> {
    if let Some(f) = opt(args, "--token-file") {
        let t = std::fs::read_to_string(&f).map_err(|e| format!("--token-file {f}: {e}"))?;
        return Ok(Some(t.trim().to_string()));
    }
    Ok(std::env::var("IGNITE_TOKEN").ok().filter(|t| !t.is_empty()))
}

/// A command's outcome: the JSON report plus the text the human CLI prints.
struct Out {
    report: Value,
    text: String,
}

/// `list`, `delete-notebook <index|id>`, `delete-page <index|id> --page N` (N is 1-based).
fn manage_cmd(args: &[String]) -> Result<Out, String> {
    use ignite_import::client::fetch_snapshot;
    use ignite_import::manage::{
        format_rows, plan_copy_pages, plan_delete_notebook, plan_delete_page, rows, Ids,
    };

    let cmd = args[0].as_str();
    let url = opt(args, "--server").ok_or("--server is required")?;
    let http = Http::new(&url, token(args)?)?;
    let library = discover_library(&http, opt(args, "--library").as_deref())?;
    let snap = fetch_snapshot(&http, library)?;
    if cmd == "list" {
        let r = rows(&snap);
        return Ok(Out {
            report: json!({
                "ok": true, "command": "list", "library": library.to_string(),
                "notebooks": r.iter().map(|r| json!({
                    "index": r.index, "id": r.id.to_string(), "title": r.title,
                    "pages": r.pages, "strokes": r.strokes
                })).collect::<Vec<_>>()
            }),
            text: format_rows(&r),
        });
    }

    let sel = args
        .get(1)
        .filter(|a| !a.starts_with("--"))
        .ok_or("give the notebook as a list index or full id (see `list`)")?;
    let write = flag(args, "--confirm-write");
    let dir = opt(args, "--state-dir").map(PathBuf::from);
    let mut mint = || Uuid::now_v7();
    let mut replica = match (&dir, write) {
        (Some(d), _) => ReplicaFile::load_or_create(d, library, &mut mint)?,
        (None, true) => return Err("--confirm-write needs --state-dir".into()),
        (None, false) => {
            // Read-only preview: a throwaway identity that is never saved.
            ReplicaFile::load_or_create(
                &std::env::temp_dir().join("ignite-import-preview"),
                library,
                &mut mint,
            )?
        }
    };
    let ids = Ids {
        library_id: library,
        replica_id: replica.replica_id,
        first_client_seq: replica.next_client_seq,
    };
    let (plan, what) = if cmd == "delete-notebook" {
        plan_delete_notebook(&snap, sel, &ids, &mut mint)?
    } else if cmd == "copy-pages" {
        let pages =
            parse_pages(&opt(args, "--pages").ok_or("copy-pages needs --pages, e.g. 1-4")?)?;
        let into = opt(args, "--into").ok_or("copy-pages needs --into <index|id>")?;
        let before = match opt(args, "--before") {
            None => None,
            Some(n) => match n.parse::<usize>() {
                Ok(n) if n >= 1 => Some(n - 1),
                _ => return Err("--before is a 1-based page number".into()),
            },
        };
        let (plan, what) = plan_copy_pages(&snap, sel, &pages, &into, before, &ids, &mut mint)?;
        if let Some(dir) = opt(args, "--preview-dir") {
            let title = ignite_import::manage::find_notebook(&snap, &into)?
                .1
                .title
                .clone();
            write_preview(Path::new(&dir), &title, &plan)?;
        }
        (plan, what)
    } else {
        let page: usize = opt(args, "--page")
            .ok_or("delete-page needs --page N (1-based)")?
            .parse()
            .map_err(|_| "--page must be a number".to_string())?;
        if page == 0 {
            return Err("--page is 1-based".into());
        }
        plan_delete_page(&snap, sel, page - 1, &ids, &mut mint)?
    };
    let mut report = json!({
        "ok": true, "command": cmd, "library": library.to_string(),
        "notebook": { "id": plan.notebook_id.to_string() },
        "description": what, "envelopes": plan.envelopes.len(), "written": false
    });
    if !write {
        return Ok(Out {
            report,
            text: if cmd == "copy-pages" {
                format!("would {what}\n{} operations; nothing was sent (pass --confirm-write and --state-dir to write)\n", plan.envelopes.len())
            } else {
                format!("would delete {what}\nnothing was sent (pass --confirm-write and --state-dir to delete)\n")
            },
        });
    }
    let res = submit(&http, &plan, &mut replica, &mut |_| {})?;
    report["written"] = json!(true);
    report["headServerSeq"] = json!(res.head_server_seq);
    Ok(Out {
        report,
        text: if cmd == "copy-pages" {
            format!("done: {what}\nThe Kindle receives the pages on its next sync pull.\n")
        } else {
            format!("deleted {what}\nThe Kindle applies the deletion on its next sync pull; deleted items are quarantined on the device, not erased.\n")
        },
    })
}

/// "1-4", "2", "1,3,5" or "1-2,5" (1-based) → 0-based indexes in the given order.
fn parse_pages(s: &str) -> Result<Vec<usize>, String> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let bad = || format!("--pages {s:?}: use e.g. 1-4 or 1,3,5 (1-based)");
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (
                a.trim().parse::<usize>().map_err(|_| bad())?,
                b.trim().parse::<usize>().map_err(|_| bad())?,
            ),
            None => {
                let n = part.trim().parse::<usize>().map_err(|_| bad())?;
                (n, n)
            }
        };
        if a == 0 || b < a {
            return Err(bad());
        }
        out.extend((a..=b).map(|n| n - 1));
    }
    Ok(out)
}

/// Snapshot-shaped pages (the same JSON the server's snapshot uses) plus the background bytes,
/// so a caller can render the plan exactly like a synced page.
fn write_preview(dir: &Path, title: &str, plan: &Plan) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (i, p) in plan.pages.iter().enumerate() {
        let f = dir.join(format!("page-{:02}.svg", i + 1));
        std::fs::write(&f, page_svg(p)).map_err(|e| format!("{}: {e}", f.display()))?;
    }
    let mut pages: Vec<Value> = Vec::new();
    let mut index = std::collections::HashMap::new();
    for e in &plan.envelopes {
        let page = match &e.operation {
            SyncOperationV1::CreateNotebook { first_page, .. } => first_page,
            SyncOperationV1::CreatePage { page, .. } => page,
            _ => continue,
        };
        index.insert(page.page_id, pages.len());
        pages.push(json!({
            "page_id": page.page_id.to_string(),
            "background": serde_json::to_value(&page.background).map_err(|e| e.to_string())?,
            "render": serde_json::to_value(&page.render).map_err(|e| e.to_string())?,
            "strokes": [], "texts": []
        }));
    }
    for e in &plan.envelopes {
        match &e.operation {
            SyncOperationV1::AddStroke {
                page_id, stroke, ..
            } => {
                pages[index[page_id]]["strokes"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::to_value(stroke).map_err(|e| e.to_string())?);
            }
            SyncOperationV1::AddText { page_id, text, .. } => {
                pages[index[page_id]]["texts"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::to_value(text).map_err(|e| e.to_string())?);
            }
            _ => {}
        }
    }
    let mut backgrounds = serde_json::Map::new();
    for b in &plan.blobs {
        let name = format!("paper-{}.gray8", b.sha256);
        std::fs::write(dir.join(&name), &b.bytes).map_err(|e| format!("{name}: {e}"))?;
        backgrounds.insert(b.sha256.to_string(), json!(name));
    }
    let v = json!({
        "v": 1,
        "page_space": { "width": CANONICAL_PAGE_WIDTH, "height": CANONICAL_PAGE_HEIGHT },
        "notebook": { "notebook_id": plan.notebook_id.to_string(), "title": title, "pages": pages },
        "backgrounds": backgrounds
    });
    let f = dir.join("preview.json");
    std::fs::write(&f, serde_json::to_vec(&v).unwrap()).map_err(|e| format!("{}: {e}", f.display()))
}

fn create_cmd(
    args: &[String],
    spec: Spec,
    mut warnings: Vec<String>,
    command: &str,
) -> Result<Out, String> {
    let write = flag(args, "--confirm-write");
    let mut text = String::new();
    let mut mint = || Uuid::now_v7();

    let http = match opt(args, "--server") {
        Some(url) => Some(Http::new(&url, token(args)?)?),
        None => None,
    };
    if write && http.is_none() {
        return Err("--confirm-write needs --server".into());
    }

    // Library facts: from the server when given (read-only), else a fixed offline stand-in.
    let (library, render, after, notes) = match &http {
        Some(h) => {
            let lib = discover_library(h, opt(args, "--library").as_deref())?;
            let t = fetch_target(h, lib)?;
            let note = format!(
                "library {lib} ({} notebooks); render profile {}",
                t.notebook_count,
                if t.render.is_some() {
                    "copied from the library"
                } else {
                    "none yet, using the sealed default"
                }
            );
            if t.render.is_none() {
                warnings.push(
                    "the library has no pages yet; using the sealed default render profile".into(),
                );
            }
            (
                lib,
                t.render.unwrap_or_else(default_render),
                t.after_notebook_id,
                note,
            )
        }
        None => (
            LibraryId::from_uuid(mint()).map_err(|e| e.to_string())?,
            default_render(),
            None,
            "offline dry run (no server): placeholder library, sealed default render profile"
                .into(),
        ),
    };
    text.push_str(&format!("{notes}\n"));

    let state_dir = opt(args, "--state-dir").map(PathBuf::from);
    let mut replica = match (&state_dir, write) {
        (Some(d), _) => Some(ReplicaFile::load_or_create(d, library, &mut mint)?),
        (None, true) => return Err("--confirm-write needs --state-dir".into()),
        (None, false) => None,
    };
    let (replica_id, first_seq) = match &replica {
        Some(r) => (r.replica_id, r.next_client_seq),
        None => (ReplicaId::from_uuid(mint()).map_err(|e| e.to_string())?, 1),
    };

    let ctx = Ctx {
        library_id: library,
        replica_id,
        first_client_seq: first_seq,
        render,
        after_notebook_id: after,
    };
    let plan = build(&spec, &ctx, &mut mint)?;
    let blob_bytes: usize = plan.blobs.iter().map(|b| b.bytes.len()).sum();
    text.push_str(&format!(
        "plan: notebook {:?} with {} pages, {} strokes, {} text objects, {} envelopes, {} background blob(s) ({} MB)\n",
        spec.notebook.title,
        plan.pages.len(),
        plan.stroke_count(),
        plan.text_count(),
        plan.envelopes.len(),
        plan.blobs.len(),
        blob_bytes / 1_000_000
    ));
    for w in &warnings {
        text.push_str(&format!("warning: {w}\n"));
    }
    let mut report = json!({
        "ok": true, "command": command, "written": false,
        "library": if http.is_some() { json!(library.to_string()) } else { Value::Null },
        "notebook": { "title": spec.notebook.title },
        "pages": plan.pages.len(), "strokes": plan.stroke_count(), "texts": plan.text_count(),
        "envelopes": plan.envelopes.len(), "blobs": plan.blobs.len(), "blobBytes": blob_bytes,
        "warnings": warnings
    });

    if let Some(dir) = opt(args, "--preview-dir") {
        write_preview(Path::new(&dir), &spec.notebook.title, &plan)?;
        text.push_str(&format!("previews written to {dir}\n"));
        report["previewDir"] = json!(dir);
    }

    if !write {
        text.push_str(
            "validated; nothing was sent (pass --confirm-write to create the notebook)\n",
        );
        return Ok(Out { report, text });
    }

    let h: &dyn Transport = http.as_ref().unwrap();
    let r = replica.as_mut().unwrap();
    let res = submit(h, &plan, r, &mut |m| eprintln!("{m}"))?;
    text.push_str(&format!(
        "created notebook {} : {} envelopes, {} background(s) uploaded, server head {}\n",
        plan.notebook_id, res.envelopes, res.blobs_uploaded, res.head_server_seq
    ));
    text.push_str(
        "The Kindle will pick it up on its next sync pull (after ~1.5 s of pen inactivity).\n",
    );
    report["written"] = json!(true);
    report["notebook"]["id"] = json!(plan.notebook_id.to_string());
    report["blobsUploaded"] = json!(res.blobs_uploaded);
    // Ink the importer drew (checkboxes, rules, labels): readers can tell it from handwriting.
    report["strokeIds"] = json!(plan
        .envelopes
        .iter()
        .filter_map(|e| match &e.operation {
            SyncOperationV1::AddStroke { stroke, .. } => Some(stroke.stroke_id.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>());
    report["headServerSeq"] = json!(res.head_server_seq);
    Ok(Out { report, text })
}

fn note_cmd(args: &[String]) -> Result<Out, String> {
    let path = args
        .get(1)
        .filter(|a| !a.starts_with("--"))
        .ok_or("note needs a Markdown file")?;
    let md = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let paper = match opt(args, "--paper").as_deref() {
        None | Some("blank") => Paper::Blank,
        Some("lined") => Paper::Lined,
        Some("dot") => Paper::Dot,
        Some("grid") => Paper::Grid,
        Some(p) => return Err(format!("--paper {p:?}: use blank, lined, dot or grid")),
    };
    let extra_pages = match opt(args, "--extra-pages") {
        None => 0,
        Some(n) => n
            .parse()
            .map_err(|_| "--extra-pages must be a number".to_string())?,
    };
    let fallback = Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Note".into());
    let (spec, warnings) = from_markdown(
        &md,
        &fallback,
        &NoteOptions {
            title: opt(args, "--title"),
            paper,
            extra_pages,
        },
    )?;
    if let Some(f) = opt(args, "--spec-out") {
        std::fs::write(&f, spec_json(&spec)).map_err(|e| format!("{f}: {e}"))?;
    }
    create_cmd(args, spec, warnings, "note")
}

/// The generated spec as page-spec JSON (canonical coordinates), for inspection or reuse.
fn spec_json(spec: &Spec) -> String {
    use ignite_import::spec::Shape;
    let pages: Vec<Value> = spec
        .pages
        .iter()
        .map(|p| {
            json!({
                "paper": format!("{:?}", p.paper).to_lowercase(),
                "shapes": p.shapes.iter().map(|s| match s {
                    Shape::Rect { x, y, w, h } => json!({"kind": "rect", "x": x, "y": y, "w": w, "h": h}),
                    Shape::Ellipse { cx, cy, rx, ry } => json!({"kind": "ellipse", "cx": cx, "cy": cy, "rx": rx, "ry": ry}),
                    Shape::Line { from, to } => json!({"kind": "line", "from": from, "to": to}),
                    Shape::Arrow { from, to } => json!({"kind": "arrow", "from": from, "to": to}),
                }).collect::<Vec<_>>(),
                "strokes": p.strokes.iter().map(|s| json!({"points": s.points})).collect::<Vec<_>>(),
                "texts": p.texts.iter().map(|t| json!({"x": t.x, "y": t.y, "width": t.width, "font_size": t.font_size, "content": t.content})).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::to_string_pretty(
        &json!({"coords": "canonical", "notebook": {"title": spec.notebook.title}, "pages": pages}),
    )
    .unwrap()
}

fn run(args: &[String]) -> Result<Out, String> {
    match args.first().map(String::as_str) {
        Some("list" | "delete-notebook" | "delete-page" | "copy-pages") => return manage_cmd(args),
        Some("note") => return note_cmd(args),
        _ => {}
    }
    let spec_path = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or_else(|| USAGE.to_string())?;
    let spec = Spec::parse(&std::fs::read(spec_path).map_err(|e| format!("{spec_path}: {e}"))?)?;
    create_cmd(args, spec, Vec::new(), "create")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let as_json = flag(&args, "--json");
    match run(&args) {
        Ok(o) => {
            if as_json {
                println!("{}", o.report);
            } else {
                print!("{}", o.text);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if as_json {
                println!("{}", json!({"ok": false, "error": e}));
            } else {
                eprintln!("{e}");
            }
            ExitCode::FAILURE
        }
    }
}
