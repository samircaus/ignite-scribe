//! `ignite-import` CLI. See the crate docs and `ignite/tools/ignite-import/README.md`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ignite_import::build::{build, default_render, Ctx, Plan};
use ignite_import::client::{discover_library, fetch_target, submit, Http, ReplicaFile, Transport};
use ignite_import::folder::{self, Target as FolderTarget};
use ignite_import::manage::Ids;
use ignite_import::note::{from_markdown, NoteOptions};
use ignite_import::preview::page_svg;
use ignite_import::spec::{Paper, Spec};
use serde_json::{json, Value};
use sync_protocol::{
    LibraryId, ReplicaId, SyncOperationV1, CANONICAL_PAGE_HEIGHT, CANONICAL_PAGE_WIDTH,
};
use uuid::Uuid;

const USAGE: &str = "\
ignite-import <spec.json> [--folder NAME] [options]
ignite-import note <note.md> [--title T] [--paper blank|lined|dot|grid] [--extra-pages N]
              [--folder NAME] [options]

  Validates a page spec (or lays out a Markdown note with the Kindle's own text metrics) and,
  with --confirm-write, creates it as a NEW notebook on an Ignite sync server. Existing
  notebooks are never modified. --folder (or the spec's notebook.folder) files the new notebook
  in that folder; a folder that does not exist yet is created (the report says so).

ignite-import list --server URL --token-file FILE
ignite-import delete-notebook <index|id> --server URL --token-file FILE --state-dir DIR [--confirm-write]
ignite-import delete-page <index|id> --page N --server URL --token-file FILE --state-dir DIR [--confirm-write]
ignite-import copy-pages <index|id> --pages 1-4 --into <index|id> [--before N] --server URL
              --token-file FILE --state-dir DIR [--preview-dir DIR] [--confirm-write]

  copy-pages copies pages verbatim (every stroke point and pressure, typed text) into another
  notebook, before its page N (default: at the end). The source notebook is not changed.

ignite-import annotate <add.json> --notebook <index|id> --page N --server URL --token-file FILE
              --state-dir DIR [--preview-dir DIR] [--confirm-write]

  annotate adds ink and typed text (one page of the spec: shapes, strokes, labels, writing,
  drawings, texts) to page N (1-based) of an existing notebook, or with new_page to a new page
  inserted right after it. Nothing already there is changed; --json reports the new ids, and
  every write is recorded in <state-dir>/ai-ink.jsonl.

ignite-import delete-ink <id>... | --last --server URL --token-file FILE --state-dir DIR
              [--confirm-write]

  delete-ink removes strokes/text that annotate wrote (ids from its report, or --last for the
  most recent annotate). Ids not in the journal are refused, so your own ink is never removed.

ignite-import measure <items.json>

  measure prints the size of handwriting before it is placed: items are
  [{ \"text\", \"size\"?, \"width\"? }] in page-width units, the answer lists { \"w\", \"h\" } each.

ignite-import move-notebook <index|id>... (--folder NAME | --top-level) --server URL
              --token-file FILE --state-dir DIR [--confirm-write]

  move-notebook files existing notebooks in a folder (by title, case-insensitive, or id;
  created when missing) or moves them back to the top level. Only folder membership changes.

  list shows notebooks with their indexes and folders. The delete commands only describe what they would
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
        format_rows, plan_copy_pages, plan_delete_notebook, plan_delete_page, rows,
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
                    "pages": r.pages, "strokes": r.strokes,
                    "folder": snap.notebooks[r.index].folder_id.and_then(|f| snap.folders.iter().find(|x| x.folder_id == f)).map(|f| f.title.clone())
                })).collect::<Vec<_>>(),
                "folders": snap.folders.iter().map(|f| json!({"id": f.folder_id.to_string(), "title": f.title})).collect::<Vec<_>>()
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

/// `annotate <add.json> --notebook <index|id> --page N`.
fn annotate_cmd(args: &[String]) -> Result<Out, String> {
    use ignite_import::annotate::{append_journal, plan_annotate, AnnotateSpec, JournalEntry};
    use ignite_import::client::fetch_snapshot;

    let path = args
        .get(1)
        .filter(|a| !a.starts_with("--"))
        .ok_or("annotate needs a JSON file with the additions")?;
    let add = AnnotateSpec::parse(&std::fs::read(path).map_err(|e| format!("{path}: {e}"))?)?;
    let sel = opt(args, "--notebook").ok_or("annotate needs --notebook <index|id>")?;
    let page = match opt(args, "--page").map(|n| n.parse::<usize>()) {
        Some(Ok(n)) if n >= 1 => n - 1,
        _ => return Err("annotate needs --page N (1-based)".into()),
    };
    let url = opt(args, "--server").ok_or("--server is required")?;
    let http = Http::new(&url, token(args)?)?;
    let library = discover_library(&http, opt(args, "--library").as_deref())?;
    let snap = fetch_snapshot(&http, library)?;
    let write = flag(args, "--confirm-write");
    let mut mint = || Uuid::now_v7();
    let mut replica = match (opt(args, "--state-dir"), write) {
        (Some(d), _) => ReplicaFile::load_or_create(Path::new(&d), library, &mut mint)?,
        (None, true) => return Err("--confirm-write needs --state-dir".into()),
        (None, false) => ReplicaFile::load_or_create(
            &std::env::temp_dir().join("ignite-import-preview"),
            library,
            &mut mint,
        )?,
    };
    let ids = Ids {
        library_id: library,
        replica_id: replica.replica_id,
        first_client_seq: replica.next_client_seq,
    };
    let (plan, added, what) = plan_annotate(&snap, &sel, page, &add, &ids, &mut mint)?;
    if let Some(dir) = opt(args, "--preview-dir") {
        // Only the additions (the page's own ink is not drawn).
        std::fs::create_dir_all(&dir).map_err(|e| format!("{dir}: {e}"))?;
        let f = Path::new(&dir).join("additions.svg");
        std::fs::write(&f, page_svg(&plan.pages[0]))
            .map_err(|e| format!("{}: {e}", f.display()))?;
        // The planned strokes and texts in snapshot form (pens included), so a caller can
        // render the page as it will look: its current ink plus these.
        let mut strokes = Vec::new();
        let mut texts = Vec::new();
        for env in &plan.envelopes {
            match &env.operation {
                SyncOperationV1::AddStroke { stroke, .. } => strokes.push(stroke),
                SyncOperationV1::AddText { text, .. } => texts.push(text),
                _ => {}
            }
        }
        let f = Path::new(&dir).join("additions.json");
        let body =
            json!({ "page_id": added.page_id.to_string(), "strokes": strokes, "texts": texts });
        std::fs::write(&f, body.to_string()).map_err(|e| format!("{}: {e}", f.display()))?;
    }
    let mut report = json!({
        "ok": true, "command": "annotate", "library": library.to_string(),
        "notebook": { "id": plan.notebook_id.to_string(), "title": added.notebook_title },
        "page": { "index": page + 1, "id": added.page_id },
        "description": what, "envelopes": plan.envelopes.len(), "written": false,
        "replicaId": replica.replica_id.to_string(),
        "pageId": added.page_id, "createdPageId": added.created_page_id,
        "strokeIds": added.stroke_ids, "textIds": added.text_ids, "linkIds": added.link_ids,
        "warnings": plan.warnings
    });
    if !write {
        return Ok(Out {
            report,
            text: format!("would {what}\n{} operations; nothing was sent (pass --confirm-write and --state-dir to write)\n", plan.envelopes.len()),
        });
    }
    let res = submit(&http, &plan, &mut replica, &mut |_| {})?;
    let dir = PathBuf::from(opt(args, "--state-dir").unwrap_or_default());
    append_journal(
        &dir,
        &JournalEntry {
            at: now_rfc3339(),
            notebook_id: added.notebook_id.clone(),
            page_id: added.page_id.clone(),
            created_page_id: added.created_page_id.clone(),
            stroke_ids: added.stroke_ids.clone(),
            text_ids: added.text_ids.clone(),
            link_ids: added.link_ids.clone(),
            head_server_seq: res.head_server_seq,
        },
    )?;
    report["written"] = json!(true);
    report["headServerSeq"] = json!(res.head_server_seq);
    Ok(Out {
        report,
        text: format!("done: {what}\nThe Kindle receives it on its next sync pull.\n"),
    })
}

/// Seconds since the epoch as an RFC 3339 UTC timestamp (no date crate needed).
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// `delete-ink <id>... | --last` : remove ink this importer wrote (journal-checked).
fn delete_ink_cmd(args: &[String]) -> Result<Out, String> {
    use ignite_import::annotate::{plan_delete_ink, read_journal};
    use ignite_import::client::fetch_snapshot;

    let dir = PathBuf::from(
        opt(args, "--state-dir")
            .ok_or("delete-ink needs --state-dir (its journal lists the ink it may remove)")?,
    );
    let journal = read_journal(&dir)?;
    let mut wanted: Vec<String> = args[1..]
        .iter()
        .take_while(|a| !a.starts_with("--"))
        .cloned()
        .collect();
    if flag(args, "--last") {
        let last = journal
            .last()
            .ok_or("the journal is empty: nothing to undo")?;
        wanted.extend(last.stroke_ids.iter().cloned());
        wanted.extend(last.text_ids.iter().cloned());
    }
    let url = opt(args, "--server").ok_or("--server is required")?;
    let http = Http::new(&url, token(args)?)?;
    let library = discover_library(&http, opt(args, "--library").as_deref())?;
    let snap = fetch_snapshot(&http, library)?;
    let mut mint = || Uuid::now_v7();
    let mut replica = ReplicaFile::load_or_create(&dir, library, &mut mint)?;
    let ids = Ids {
        library_id: library,
        replica_id: replica.replica_id,
        first_client_seq: replica.next_client_seq,
    };
    let (plan, removal, what) = plan_delete_ink(&snap, &journal, &wanted, &ids, &mut mint)?;
    let mut report = json!({
        "ok": true, "command": "delete-ink", "library": library.to_string(),
        "description": what, "envelopes": plan.envelopes.len(), "written": false,
        "strokes": removal.strokes, "texts": removal.texts, "alreadyGone": removal.already_gone
    });
    if !flag(args, "--confirm-write") || plan.envelopes.is_empty() {
        return Ok(Out {
            report,
            text: format!("would {what}\nnothing was sent (pass --confirm-write to remove)\n"),
        });
    }
    let res = submit(&http, &plan, &mut replica, &mut |_| {})?;
    report["written"] = json!(true);
    report["headServerSeq"] = json!(res.head_server_seq);
    Ok(Out {
        report,
        text: format!("done: {what}\nThe Kindle applies it on its next sync pull.\n"),
    })
}

/// `move-notebook <index|id>... (--folder NAME | --top-level)`.
fn move_cmd(args: &[String]) -> Result<Out, String> {
    use ignite_import::client::fetch_snapshot;

    let url = opt(args, "--server").ok_or("--server is required")?;
    let http = Http::new(&url, token(args)?)?;
    let library = discover_library(&http, opt(args, "--library").as_deref())?;
    let snap = fetch_snapshot(&http, library)?;
    let sels: Vec<String> = args[1..]
        .iter()
        .take_while(|a| !a.starts_with("--"))
        .cloned()
        .collect();
    let mut mint = || Uuid::now_v7();
    let target = match (opt(args, "--folder"), flag(args, "--top-level")) {
        (Some(name), false) => folder::resolve(&snap.folders, &name, &mut mint)?,
        (None, true) => FolderTarget::TopLevel,
        _ => return Err("move-notebook needs exactly one of --folder NAME or --top-level".into()),
    };
    let write = flag(args, "--confirm-write");
    let dir = opt(args, "--state-dir").map(PathBuf::from);
    let mut replica = match (&dir, write) {
        (Some(d), _) => ReplicaFile::load_or_create(d, library, &mut mint)?,
        (None, true) => return Err("--confirm-write needs --state-dir".into()),
        (None, false) => ReplicaFile::load_or_create(
            &std::env::temp_dir().join("ignite-import-preview"),
            library,
            &mut mint,
        )?,
    };
    let ids = Ids {
        library_id: library,
        replica_id: replica.replica_id,
        first_client_seq: replica.next_client_seq,
    };
    let (plan, moved, unchanged, what) = folder::plan_move(&snap, &sels, &target, &ids, &mut mint)?;
    let list = |v: &[folder::Moved]| {
        v.iter()
            .map(|m| json!({"id": m.id.to_string(), "title": m.title, "from": m.from}))
            .collect::<Vec<_>>()
    };
    let mut report = json!({
        "ok": true, "command": "move-notebook", "library": library.to_string(),
        "notebook": { "id": plan.notebook_id.to_string() },
        "folder": target.report(),
        "notebooks": list(&moved), "unchanged": list(&unchanged),
        "description": what, "envelopes": plan.envelopes.len(), "written": false
    });
    if !write {
        return Ok(Out {
            report,
            text: format!("would {what}\n{} operations; nothing was sent (pass --confirm-write and --state-dir to write)\n", plan.envelopes.len()),
        });
    }
    let res = submit(&http, &plan, &mut replica, &mut |_| {})?;
    report["written"] = json!(true);
    report["headServerSeq"] = json!(res.head_server_seq);
    Ok(Out {
        report,
        text: format!("done: {what}\nThe Kindle applies it on its next sync pull.\n"),
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
    let (library, render, after, notes, folders) = match &http {
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
                t.folders,
            )
        }
        None => (
            LibraryId::from_uuid(mint()).map_err(|e| e.to_string())?,
            default_render(),
            None,
            "offline dry run (no server): placeholder library, sealed default render profile"
                .into(),
            Vec::new(),
        ),
    };
    let folder = match spec.notebook.folder.as_deref() {
        None => None,
        Some(name) => {
            let t = folder::resolve(&folders, name, &mut mint)?;
            if http.is_none() {
                warnings.push(format!(
                    "offline: folder {:?} was not checked against the library",
                    name.trim()
                ));
            } else if let FolderTarget::New { title, .. } = &t {
                warnings.push(format!(
                    "folder {title:?} does not exist yet; this write creates it"
                ));
            }
            Some(t)
        }
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
    let mut plan = build(&spec, &ctx, &mut mint)?;
    if let Some(t) = &folder {
        let ids = Ids {
            library_id: library,
            replica_id,
            first_client_seq: first_seq,
        };
        folder::file_new_notebook(&mut plan, t, &ids, &mut mint)?;
    }
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
    if let Some(t) = &folder {
        text.push_str(&format!("filed in {}\n", t.describe()));
    }
    for w in &warnings {
        text.push_str(&format!("warning: {w}\n"));
    }
    let mut report = json!({
        "ok": true, "command": command, "written": false,
        "library": if http.is_some() { json!(library.to_string()) } else { Value::Null },
        "notebook": { "title": spec.notebook.title },
        "folder": folder.as_ref().map(FolderTarget::report).unwrap_or(Value::Null),
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
    let (mut spec, warnings) = from_markdown(
        &md,
        &fallback,
        &NoteOptions {
            title: opt(args, "--title"),
            paper,
            extra_pages,
        },
    )?;
    spec.notebook.folder = opt(args, "--folder");
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
        &json!({"coords": "canonical", "notebook": {"title": spec.notebook.title, "folder": spec.notebook.folder}, "pages": pages}),
    )
    .unwrap()
}

/// `measure <items.json>`: size of handwriting before placing it. Items are
/// `[{ "text", "size"?, "width"? }]` in page-width units; the answer lists `{ "w", "h" }` for each
/// (h from the top of the capitals to the lowest descender). Nothing is contacted.
fn measure_cmd(args: &[String]) -> Result<Out, String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Item {
        text: String,
        #[serde(default)]
        size: Option<f32>,
        #[serde(default)]
        width: Option<f32>,
    }
    let path = args
        .get(1)
        .filter(|a| !a.starts_with("--"))
        .ok_or("measure needs <items.json>")?;
    let raw = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let items: Vec<Item> = serde_json::from_slice(&raw).map_err(|e| format!("{path}: {e}"))?;
    let mut sizes = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let missing = ignite_import::hand::missing(&it.text);
        if !missing.is_empty() {
            let list: String = missing.iter().collect();
            return Err(format!(
                "item {}: the handwriting font lacks {list:?}",
                i + 1
            ));
        }
        let size = it.size.unwrap_or(ignite_import::spec::WRITING_SIZE);
        let (w, h) = ignite_import::hand::measure(&it.text, size, it.width);
        sizes.push(json!({ "w": w, "h": h }));
    }
    let report = json!({ "ok": true, "command": "measure", "sizes": sizes });
    Ok(Out {
        text: format!("{report}\n"),
        report,
    })
}

fn run(args: &[String]) -> Result<Out, String> {
    match args.first().map(String::as_str) {
        Some("list" | "delete-notebook" | "delete-page" | "copy-pages") => return manage_cmd(args),
        Some("note") => return note_cmd(args),
        Some("move-notebook") => return move_cmd(args),
        Some("annotate") => return annotate_cmd(args),
        Some("delete-ink") => return delete_ink_cmd(args),
        Some("measure") => return measure_cmd(args),
        _ => {}
    }
    let spec_path = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or_else(|| USAGE.to_string())?;
    let mut spec =
        Spec::parse(&std::fs::read(spec_path).map_err(|e| format!("{spec_path}: {e}"))?)?;
    if let Some(f) = opt(args, "--folder") {
        spec.notebook.folder = Some(f);
    }
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
