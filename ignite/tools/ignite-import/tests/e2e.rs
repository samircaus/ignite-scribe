//! End-to-end: a real in-process `scribe-sync-server` (real HTTP, real SQLite). The importer
//! creates a notebook as a second replica; we then read the server's reducer-built snapshot
//! back and check exactly what a Kindle/web replica would receive.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ignite_import::build::{build, Ctx};
use ignite_import::client::{discover_library, fetch_target, submit, Http, ReplicaFile, Transport};
use ignite_import::spec::Spec;
use scribe_sync_server::{ServerHandle, Store};
use sync_engine::testkit::{library, nb, pg, render};
use sync_protocol::{
    BackgroundRef, PageSpace, SnapshotNotebook, SnapshotPage, StrokeStyle, SyncSnapshotV1,
};
use uuid::Uuid;

static N: AtomicU64 = AtomicU64::new(0);

fn tmp(label: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!(
        "ignite-import-{label}-{n}-{}",
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&p);
    p
}

/// One notebook, one blank page, no strokes: the smallest valid library.
fn genesis() -> SyncSnapshotV1 {
    SyncSnapshotV1 {
        v: 1,
        library_id: library(),
        page_space: PageSpace::CANONICAL,
        notebooks: vec![SnapshotNotebook {
            notebook_id: nb(1),
            title: "Existing notebook".into(),
            pages: vec![SnapshotPage {
                page_id: pg(1),
                background: BackgroundRef::Blank,
                render: render(true),
                strokes: vec![],
                texts: vec![],
                links: vec![],
            }],
            folder_id: None,
        }],
        folders: vec![],
    }
}

const SPEC: &str = r#"{
  "notebook": {"title": "Imported"},
  "pages": [
    {"paper": "lined",
     "labels": [{"text": "Hi", "x": 0.1, "y": 0.1, "size": 0.04}],
     "shapes": [{"kind": "rect", "x": 0.1, "y": 0.3, "w": 0.3, "h": 0.1}],
     "texts": [{"x": 0.1, "y": 0.6, "width": 0.5, "font_size": 0.03, "content": "note"}]},
    {"paper": "lined", "strokes": [{"style": "pressure", "points": [[0.2,0.2,0.1],[0.6,0.4,0.9]]}]}
  ]}"#;

fn snapshot(h: &Http, lib: sync_protocol::LibraryId) -> SyncSnapshotV1 {
    let (s, b) = h
        .request("GET", &format!("/v1/libraries/{lib}/snapshot"), b"")
        .unwrap();
    assert_eq!(s, 200);
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    let snap = SyncSnapshotV1::parse(v["snapshot"].to_string().as_bytes()).unwrap();
    snap
}

#[test]
fn imports_a_notebook_and_a_second_run_appends_without_touching_the_first() {
    let dir = tmp("srv");
    let server =
        ServerHandle::start("127.0.0.1:0".parse().unwrap(), Store::open(&dir).unwrap()).unwrap();
    let http = Http::new(&server.base_url(), None).unwrap();

    let g = genesis();
    let (s, b) = http
        .request("POST", "/v1/libraries", &g.to_canonical_bytes().unwrap())
        .unwrap();
    assert!(s == 200 || s == 201, "{s} {}", String::from_utf8_lossy(&b));

    // Discovery + target facts come from the live library.
    let lib = discover_library(&http, None).unwrap();
    assert_eq!(lib, library());
    let target = fetch_target(&http, lib).unwrap();
    assert_eq!(target.notebook_count, 1);
    assert_eq!(target.after_notebook_id, Some(nb(1)));
    assert!(target.render.is_some());

    let state = tmp("state");
    let spec = Spec::parse(SPEC.as_bytes()).unwrap();
    let mut mint = || Uuid::now_v7();

    let mut totals = Vec::new();
    for round in 0..2 {
        let target = fetch_target(&http, lib).unwrap();
        let mut replica = ReplicaFile::load_or_create(&state, lib, &mut mint).unwrap();
        assert_eq!(replica.next_client_seq, 1 + totals.iter().sum::<u64>());
        let ctx = Ctx {
            library_id: lib,
            replica_id: replica.replica_id,
            first_client_seq: replica.next_client_seq,
            render: target.render.clone().unwrap(),
            after_notebook_id: target.after_notebook_id,
        };
        let plan = build(&spec, &ctx, &mut mint).unwrap();
        let res = submit(&http, &plan, &mut replica, &mut |_| {}).unwrap();
        assert_eq!(res.envelopes, plan.envelopes.len());
        assert_eq!(res.blobs_uploaded, 1, "both pages share one lined blob");
        totals.push(plan.envelopes.len() as u64);

        // The server's replayed snapshot is the truth every replica converges on.
        let snap = snapshot(&http, lib);
        assert_eq!(snap.notebooks.len(), 2 + round);
        let ours = snap
            .notebooks
            .iter()
            .find(|n| n.notebook_id == plan.notebook_id)
            .expect("imported notebook present");
        assert_eq!(ours.title, "Imported");
        assert_eq!(ours.pages.len(), 2);
        assert!(matches!(
            ours.pages[0].background,
            BackgroundRef::Blob { .. }
        ));
        assert_eq!(ours.pages[0].strokes.len(), plan.pages[0].strokes.len());
        assert_eq!(ours.pages[0].texts.len(), 1);
        assert_eq!(ours.pages[1].strokes.len(), 1);
        assert_eq!(ours.pages[1].strokes[0].style, StrokeStyle::PressureWidthV1);
        // Blob is retrievable by hash (what the Kindle fetches during inbound).
        if let BackgroundRef::Blob {
            sha256, byte_len, ..
        } = &ours.pages[0].background
        {
            let (s, body) = http
                .request("GET", &format!("/v1/blobs/{sha256}"), b"")
                .unwrap();
            assert_eq!(s, 200);
            assert_eq!(body.len() as u64, *byte_len);
        }

        // The pre-existing notebook is byte-for-byte unchanged.
        let first = &snap.notebooks[0];
        assert_eq!(first, &g.notebooks[0], "existing notebook must not change");
    }
}

#[test]
fn rerunning_the_same_envelopes_is_idempotent_on_the_server() {
    let dir = tmp("srv2");
    let server =
        ServerHandle::start("127.0.0.1:0".parse().unwrap(), Store::open(&dir).unwrap()).unwrap();
    let http = Http::new(&server.base_url(), None).unwrap();
    http.request(
        "POST",
        "/v1/libraries",
        &genesis().to_canonical_bytes().unwrap(),
    )
    .unwrap();
    let lib = library();
    let target = fetch_target(&http, lib).unwrap();
    let state = tmp("state2");
    let spec = Spec::parse(SPEC.as_bytes()).unwrap();
    let mut mint = || Uuid::now_v7();
    let mut replica = ReplicaFile::load_or_create(&state, lib, &mut mint).unwrap();
    let ctx = Ctx {
        library_id: lib,
        replica_id: replica.replica_id,
        first_client_seq: 1,
        render: target.render.unwrap(),
        after_notebook_id: target.after_notebook_id,
    };
    let plan = build(&spec, &ctx, &mut mint).unwrap();
    submit(&http, &plan, &mut replica, &mut |_| {}).unwrap();

    // A lost ack would make the client resend identical bytes: the server must deduplicate.
    let first = plan.envelopes[0].to_canonical_bytes().unwrap();
    let mut body = b"{\"envelopes\":[".to_vec();
    body.extend(&first);
    body.extend(b"]}");
    let (s, resp) = http
        .request("POST", &format!("/v1/libraries/{lib}/operations"), &body)
        .unwrap();
    assert_eq!(s, 200, "{}", String::from_utf8_lossy(&resp));
    let v: serde_json::Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(v["results"][0]["duplicate"], true);
    assert_eq!(
        snapshot(&http, lib).notebooks.len(),
        2,
        "no second notebook"
    );
}

#[test]
fn a_stale_state_file_is_refused_not_silently_reused() {
    let state = tmp("state3");
    let mut mint = || Uuid::now_v7();
    let a = sync_protocol::LibraryId::from_uuid(mint()).unwrap();
    let b = sync_protocol::LibraryId::from_uuid(mint()).unwrap();
    let r = ReplicaFile::load_or_create(&state, a, &mut mint).unwrap();
    r.save().unwrap();
    let e = ReplicaFile::load_or_create(&state, b, &mut mint).unwrap_err();
    assert!(e.contains("another library"), "{e}");
}

#[test]
fn delete_page_and_notebook_leave_everything_else_untouched() {
    use ignite_import::client::fetch_snapshot;
    use ignite_import::manage::{plan_delete_notebook, plan_delete_page, Ids};

    let dir = tmp("srv-del");
    let server =
        ServerHandle::start("127.0.0.1:0".parse().unwrap(), Store::open(&dir).unwrap()).unwrap();
    let http = Http::new(&server.base_url(), None).unwrap();
    http.request(
        "POST",
        "/v1/libraries",
        &genesis().to_canonical_bytes().unwrap(),
    )
    .unwrap();
    let lib = library();
    let state = tmp("state-del");
    let spec = Spec::parse(SPEC.as_bytes()).unwrap();
    let mut mint = || Uuid::now_v7();
    let mut replica = ReplicaFile::load_or_create(&state, lib, &mut mint).unwrap();
    let target = fetch_target(&http, lib).unwrap();
    let ctx = Ctx {
        library_id: lib,
        replica_id: replica.replica_id,
        first_client_seq: replica.next_client_seq,
        render: target.render.unwrap(),
        after_notebook_id: target.after_notebook_id,
    };
    let plan = build(&spec, &ctx, &mut mint).unwrap();
    submit(&http, &plan, &mut replica, &mut |_| {}).unwrap();

    // Delete page 2 of the imported notebook (index 1 in the list).
    let snap = fetch_snapshot(&http, lib).unwrap();
    assert_eq!(snap.notebooks[1].pages.len(), 2);
    let ids = |r: &ReplicaFile| Ids {
        library_id: lib,
        replica_id: r.replica_id,
        first_client_seq: r.next_client_seq,
    };
    let (p, _) = plan_delete_page(&snap, "1", 1, &ids(&replica), &mut mint).unwrap();
    submit(&http, &p, &mut replica, &mut |_| {}).unwrap();
    let snap = fetch_snapshot(&http, lib).unwrap();
    assert_eq!(snap.notebooks[1].pages.len(), 1);
    assert_eq!(snap.notebooks[0], genesis().notebooks[0]);

    // Deleting the last page is refused; deleting the notebook works.
    assert!(plan_delete_page(&snap, "1", 0, &ids(&replica), &mut mint).is_err());
    let (p, desc) = plan_delete_notebook(&snap, "1", &ids(&replica), &mut mint).unwrap();
    assert!(desc.contains("Imported"));
    submit(&http, &p, &mut replica, &mut |_| {}).unwrap();
    let snap = fetch_snapshot(&http, lib).unwrap();
    assert_eq!(snap.notebooks.len(), 1);
    assert_eq!(snap.notebooks[0], genesis().notebooks[0]);
}

#[test]
fn copy_pages_inserts_verbatim_copies_before_a_page_and_leaves_the_source_alone() {
    use ignite_import::client::fetch_snapshot;
    use ignite_import::manage::{plan_copy_pages, Ids};

    let dir = tmp("srv-copy");
    let server =
        ServerHandle::start("127.0.0.1:0".parse().unwrap(), Store::open(&dir).unwrap()).unwrap();
    let http = Http::new(&server.base_url(), None).unwrap();
    http.request(
        "POST",
        "/v1/libraries",
        &genesis().to_canonical_bytes().unwrap(),
    )
    .unwrap();
    let lib = library();
    let state = tmp("state-copy");
    let spec = Spec::parse(SPEC.as_bytes()).unwrap();
    let mut mint = || Uuid::now_v7();
    let mut replica = ReplicaFile::load_or_create(&state, lib, &mut mint).unwrap();
    let target = fetch_target(&http, lib).unwrap();
    let ctx = Ctx {
        library_id: lib,
        replica_id: replica.replica_id,
        first_client_seq: replica.next_client_seq,
        render: target.render.unwrap(),
        after_notebook_id: target.after_notebook_id,
    };
    let plan = build(&spec, &ctx, &mut mint).unwrap();
    submit(&http, &plan, &mut replica, &mut |_| {}).unwrap();
    let before = fetch_snapshot(&http, lib).unwrap();
    let source = before.notebooks[1].clone();

    // Both imported pages go in front of the existing notebook's only page.
    let ids = Ids {
        library_id: lib,
        replica_id: replica.replica_id,
        first_client_seq: replica.next_client_seq,
    };
    let (p, desc) = plan_copy_pages(&before, "1", &[0, 1], "0", Some(0), &ids, &mut mint).unwrap();
    assert!(desc.contains("before page 1"), "{desc}");
    submit(&http, &p, &mut replica, &mut |_| {}).unwrap();

    let after = fetch_snapshot(&http, lib).unwrap();
    let dest = &after.notebooks[0];
    assert_eq!(dest.pages.len(), 3);
    assert_eq!(
        dest.pages[2],
        genesis().notebooks[0].pages[0],
        "original page stays last, unchanged"
    );
    for (copy, orig) in dest.pages[..2].iter().zip(&source.pages) {
        assert_ne!(copy.page_id, orig.page_id);
        assert_eq!(copy.background, orig.background);
        assert_eq!(copy.render, orig.render);
        assert_eq!(copy.strokes.len(), orig.strokes.len());
        for (c, o) in copy.strokes.iter().zip(&orig.strokes) {
            assert_ne!(c.stroke_id, o.stroke_id);
            assert_eq!(c.root_id, c.stroke_id);
            assert_eq!(c.style, o.style);
            assert_eq!(c.points, o.points, "points and pressure copied exactly");
        }
        assert_eq!(
            copy.texts.iter().map(|t| &t.content).collect::<Vec<_>>(),
            orig.texts.iter().map(|t| &t.content).collect::<Vec<_>>()
        );
    }
    assert_eq!(after.notebooks[1], source, "source notebook untouched");
}
