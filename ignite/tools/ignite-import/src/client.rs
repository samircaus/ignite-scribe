//! Talking to a `scribe-sync-server`: library discovery, blob upload, batched envelope POSTs and
//! the persistent replica identity (`replica_id` + next `client_seq`).
//!
//! Plain HTTP only (matches the server: trusted LAN). The bearer token is never logged.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use sync_protocol::{
    LibraryId, NotebookId, RenderProfileRef, ReplicaId, SnapshotFolder, SyncSnapshotV1,
};
use uuid::Uuid;

use crate::build::Plan;

/// The server's batch limits (`scribe-sync-server/src/http.rs`: `MAX_BATCH`, `MAX_JSON_BODY`).
const MAX_BATCH: usize = 100;
const MAX_BODY_BYTES: usize = 3 * 1024 * 1024;
const RETRIES: usize = 3;

pub trait Transport {
    /// `Ok((status, body))` for any HTTP response; `Err` only for transport failures.
    fn request(&self, method: &str, path: &str, body: &[u8]) -> Result<(u16, Vec<u8>), String>;
}

pub struct Http {
    base: String,
    token: Option<String>,
    agent: ureq::Agent,
}

impl Http {
    pub fn new(base: &str, token: Option<String>) -> Result<Self, String> {
        let base = base.trim_end_matches('/').to_string();
        if !base.starts_with("http://") {
            return Err(format!(
                "server URL {base:?} must start with http:// (the Ignite server speaks plain HTTP on a trusted LAN)"
            ));
        }
        Ok(Self {
            base,
            token,
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(60))
                .build(),
        })
    }
}

impl Transport for Http {
    fn request(&self, method: &str, path: &str, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
        let mut req = self.agent.request(method, &format!("{}{path}", self.base));
        if let Some(t) = &self.token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        let res = if method == "GET" {
            req.call()
        } else {
            let ct = if path.starts_with("/v1/blobs/") {
                "application/octet-stream"
            } else {
                "application/json"
            };
            req.set("Content-Type", ct).send_bytes(body)
        };
        let resp = match res {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(e) => return Err(format!("transport: {e}")),
        };
        let status = resp.status();
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut resp.into_reader(), &mut buf)
            .map_err(|e| format!("transport: reading response: {e}"))?;
        Ok((status, buf))
    }
}

fn call_json(t: &dyn Transport, method: &str, path: &str, body: &[u8]) -> Result<Value, String> {
    let (status, bytes) = t.request(method, path, body)?;
    if !(200..300).contains(&status) {
        return Err(describe(status, method, path, &bytes));
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("{method} {path}: bad JSON: {e}"))
}

fn describe(status: u16, method: &str, path: &str, body: &[u8]) -> String {
    let hint = match status {
        401 | 403 => " (check the bearer token)",
        409 => " (sequence/identity conflict: see --state-dir note in the README)",
        413 => " (request too large)",
        422 => " (the server refused the content)",
        _ => "",
    };
    format!(
        "server {status} for {method} {path}{hint}: {}",
        String::from_utf8_lossy(body)
    )
}

/// What the importer needs to know about the target library.
#[derive(Clone, Debug)]
pub struct Target {
    pub library_id: LibraryId,
    /// Render profile of an existing page (so new pages match the device's ink/polarity).
    pub render: Option<RenderProfileRef>,
    /// The last notebook, so the new one is placed at the end.
    pub after_notebook_id: Option<NotebookId>,
    pub notebook_count: usize,
    /// Live folders, for filing the new notebook.
    pub folders: Vec<SnapshotFolder>,
}

pub fn discover_library(t: &dyn Transport, explicit: Option<&str>) -> Result<LibraryId, String> {
    if let Some(id) = explicit {
        return LibraryId::parse(id).map_err(|e| e.to_string());
    }
    let libs = call_json(t, "GET", "/v1/libraries", b"")?;
    let arr = libs["libraries"].as_array().cloned().unwrap_or_default();
    match arr.len() {
        1 => LibraryId::parse(arr[0]["library_id"].as_str().unwrap_or_default())
            .map_err(|e| e.to_string()),
        0 => Err(
            "the server has no library yet: open Ignite on the Kindle, enable sync and let it \
             upload once, then retry"
                .into(),
        ),
        n => Err(format!(
            "the server has {n} libraries; pass --library <id> (ids: {})",
            arr.iter()
                .filter_map(|l| l["library_id"].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

pub fn fetch_snapshot(t: &dyn Transport, library: LibraryId) -> Result<SyncSnapshotV1, String> {
    let v = call_json(t, "GET", &format!("/v1/libraries/{library}/snapshot"), b"")?;
    serde_json::from_value(v["snapshot"].clone())
        .map_err(|e| format!("server snapshot did not parse: {e}"))
}

pub fn fetch_target(t: &dyn Transport, library: LibraryId) -> Result<Target, String> {
    let snap = fetch_snapshot(t, library)?;
    let render = snap
        .notebooks
        .iter()
        .rev()
        .flat_map(|n| n.pages.iter())
        .map(|p| p.render.clone())
        .next();
    Ok(Target {
        library_id: library,
        render,
        after_notebook_id: snap.notebooks.last().map(|n| n.notebook_id),
        notebook_count: snap.notebooks.len(),
        folders: snap.folders.clone(),
    })
}

// ---------------------------------------------------------------- replica identity

/// The importer's own replica identity and next `client_seq` (`client_seq` must be dense).
#[derive(Clone, Debug)]
pub struct ReplicaFile {
    path: PathBuf,
    pub library_id: LibraryId,
    pub replica_id: ReplicaId,
    pub next_client_seq: u64,
}

const REPLICA_FILE: &str = "ignite-import-replica.json";

impl ReplicaFile {
    pub fn load_or_create(
        dir: &Path,
        library: LibraryId,
        mint: &mut dyn FnMut() -> Uuid,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("state dir {}: {e}", dir.display()))?;
        let path = dir.join(REPLICA_FILE);
        match std::fs::read(&path) {
            Ok(b) => {
                let v: Value = serde_json::from_slice(&b)
                    .map_err(|e| format!("{}: not valid JSON: {e}", path.display()))?;
                if v["library_id"].as_str() != Some(&library.to_string()) {
                    return Err(format!(
                        "{} belongs to another library; use a separate --state-dir",
                        path.display()
                    ));
                }
                let replica_id = ReplicaId::parse(v["replica_id"].as_str().unwrap_or_default())
                    .map_err(|e| e.to_string())?;
                let next_client_seq = v["next_client_seq"]
                    .as_u64()
                    .filter(|n| *n >= 1)
                    .ok_or_else(|| format!("{}: bad next_client_seq", path.display()))?;
                Ok(Self {
                    path,
                    library_id: library,
                    replica_id,
                    next_client_seq,
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                path,
                library_id: library,
                replica_id: ReplicaId::from_uuid(mint()).map_err(|e| e.to_string())?,
                next_client_seq: 1,
            }),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let v = serde_json::json!({
            "v": 1,
            "library_id": self.library_id.to_string(),
            "replica_id": self.replica_id.to_string(),
            "next_client_seq": self.next_client_seq,
        });
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&v).unwrap())
            .and_then(|_| std::fs::rename(&tmp, &self.path))
            .map_err(|e| format!("saving {}: {e}", self.path.display()))
    }
}

// ---------------------------------------------------------------- submit

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submitted {
    pub envelopes: usize,
    pub blobs_uploaded: usize,
    pub head_server_seq: i64,
}

/// Upload blobs, then post envelopes in dense order. Each acknowledged batch advances and saves
/// the replica's `next_client_seq` immediately, so a failure part-way leaves honest state.
/// A batch is retried verbatim on transport/5xx failure: the server deduplicates by operation id.
pub fn submit(
    t: &dyn Transport,
    plan: &Plan,
    replica: &mut ReplicaFile,
    progress: &mut dyn FnMut(&str),
) -> Result<Submitted, String> {
    let library = replica.library_id;
    let mut uploaded = 0;
    for b in &plan.blobs {
        let path = format!("/v1/blobs/{}", b.sha256);
        let (status, body) = with_retries(t, "PUT", &path, &b.bytes)?;
        if status != 200 && status != 201 {
            return Err(describe(status, "PUT", &path, &body));
        }
        uploaded += 1;
        progress(&format!("uploaded background {}", b.sha256));
    }

    let canonical: Vec<Vec<u8>> = plan
        .envelopes
        .iter()
        .map(|e| e.to_canonical_bytes().map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    if plan.envelopes.first().map(|e| e.client_seq) != Some(replica.next_client_seq) {
        return Err(format!(
            "plan starts at client_seq {:?} but the replica expects {}",
            plan.envelopes.first().map(|e| e.client_seq),
            replica.next_client_seq
        ));
    }

    let (mut i, mut head, mut acked) = (0usize, 0i64, 0usize);
    while i < canonical.len() {
        let mut j = i;
        let mut bytes = 0usize;
        while j < canonical.len()
            && j - i < MAX_BATCH
            && (j == i || bytes + canonical[j].len() < MAX_BODY_BYTES)
        {
            bytes += canonical[j].len() + 1;
            j += 1;
        }
        let mut body = b"{\"envelopes\":[".to_vec();
        for (k, e) in canonical[i..j].iter().enumerate() {
            if k > 0 {
                body.push(b',');
            }
            body.extend_from_slice(e);
        }
        body.extend_from_slice(b"]}");
        let path = format!("/v1/libraries/{library}/operations");
        let (status, resp) = with_retries(t, "POST", &path, &body).map_err(|e| {
            format!(
                "{e} -- {acked} of {} envelopes were acknowledged before this",
                canonical.len()
            )
        })?;
        if status != 200 {
            return Err(format!(
                "{} -- {acked} of {} envelopes were acknowledged before this",
                describe(status, "POST", &path, &resp),
                canonical.len()
            ));
        }
        let v: Value = serde_json::from_slice(&resp).map_err(|e| format!("bad ack JSON: {e}"))?;
        if v["results"].as_array().map(Vec::len) != Some(j - i) {
            return Err(format!(
                "ack for {} envelopes did not list {} results",
                j - i,
                j - i
            ));
        }
        head = v["head_server_seq"].as_i64().unwrap_or(head);
        acked += j - i;
        replica.next_client_seq += (j - i) as u64;
        replica.save()?;
        progress(&format!("posted {acked}/{} envelopes", canonical.len()));
        i = j;
    }
    Ok(Submitted {
        envelopes: acked,
        blobs_uploaded: uploaded,
        head_server_seq: head,
    })
}

fn with_retries(
    t: &dyn Transport,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<(u16, Vec<u8>), String> {
    let mut last = String::new();
    for attempt in 0..RETRIES {
        match t.request(method, path, body) {
            Ok((s, b)) if s < 500 => return Ok((s, b)),
            Ok((s, b)) => last = format!("server {s}: {}", String::from_utf8_lossy(&b)),
            Err(e) => last = e,
        }
        if attempt + 1 < RETRIES {
            std::thread::sleep(Duration::from_millis(400 * (attempt as u64 + 1)));
        }
    }
    Err(format!(
        "{method} {path} failed after {RETRIES} attempts: {last}"
    ))
}
