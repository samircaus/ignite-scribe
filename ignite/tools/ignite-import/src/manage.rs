//! Listing and deleting notebooks/pages through the same sync operations the apps use
//! (`DeleteNotebook`, `DeletePage`). Deleting is destructive for that item only: every replica
//! (the Kindle quarantines deleted notebooks/pages rather than erasing them) converges on the
//! deletion, so these commands require an explicit selector and `--confirm-write`.

use sync_protocol::{
    LibraryId, NotebookId, OperationId, PageId, PageSpec, ReplicaId, SnapshotNotebook, StrokeId,
    SyncEnvelopeV1, SyncOperationV1, SyncSnapshotV1, SyncStroke, TextId, ENVELOPE_VERSION,
};
use uuid::Uuid;

use crate::build::Plan;

pub struct Row {
    pub index: usize,
    pub id: NotebookId,
    pub title: String,
    pub pages: usize,
    pub strokes: usize,
}

pub fn rows(snap: &SyncSnapshotV1) -> Vec<Row> {
    snap.notebooks
        .iter()
        .enumerate()
        .map(|(index, n)| Row {
            index,
            id: n.notebook_id,
            title: n.title.clone(),
            pages: n.pages.len(),
            strokes: n.pages.iter().map(|p| p.strokes.len()).sum(),
        })
        .collect()
}

pub fn format_rows(rows: &[Row]) -> String {
    let mut s = String::new();
    for r in rows {
        s.push_str(&format!(
            "[{}] {}  {:?}  pages={} strokes={}\n",
            r.index, r.id, r.title, r.pages, r.strokes
        ));
    }
    if rows.is_empty() {
        s.push_str("(no notebooks)\n");
    }
    s
}

/// Resolve `sel` as a 0-based list index or a full notebook UUID.
pub fn find_notebook<'a>(
    snap: &'a SyncSnapshotV1,
    sel: &str,
) -> Result<(usize, &'a SnapshotNotebook), String> {
    if let Ok(i) = sel.parse::<usize>() {
        return snap.notebooks.get(i).map(|n| (i, n)).ok_or_else(|| {
            format!(
                "no notebook at index {i} (library has {})",
                snap.notebooks.len()
            )
        });
    }
    let id = NotebookId::parse(sel).map_err(|e| format!("notebook selector {sel:?}: {e}"))?;
    snap.notebooks
        .iter()
        .enumerate()
        .find(|(_, n)| n.notebook_id == id)
        .map(|(i, n)| (i, n))
        .ok_or_else(|| format!("no notebook with id {id}"))
}

pub struct Ids {
    pub library_id: LibraryId,
    pub replica_id: ReplicaId,
    pub first_client_seq: u64,
}

pub fn envelope(
    ids: &Ids,
    seq: u64,
    mint: &mut dyn FnMut() -> Uuid,
    op: SyncOperationV1,
) -> Result<SyncEnvelopeV1, String> {
    let env = SyncEnvelopeV1 {
        v: ENVELOPE_VERSION,
        operation_id: OperationId::from_uuid(mint()).map_err(|e| e.to_string())?,
        library_id: ids.library_id,
        replica_id: ids.replica_id,
        client_seq: seq,
        operation: op,
    };
    env.validate().map_err(|e| e.to_string())?;
    Ok(env)
}

fn plan_of(notebook_id: NotebookId, envelopes: Vec<SyncEnvelopeV1>) -> Plan {
    Plan {
        notebook_id,
        envelopes,
        blobs: Vec::new(),
        pages: Vec::new(),
    }
}

/// Delete one whole notebook. Returns the plan and a human description of what goes.
pub fn plan_delete_notebook(
    snap: &SyncSnapshotV1,
    sel: &str,
    ids: &Ids,
    mint: &mut dyn FnMut() -> Uuid,
) -> Result<(Plan, String), String> {
    let (_, nb) = find_notebook(snap, sel)?;
    let strokes: usize = nb.pages.iter().map(|p| p.strokes.len()).sum();
    let desc = format!(
        "notebook {:?} ({}), {} pages, {} strokes",
        nb.title,
        nb.notebook_id,
        nb.pages.len(),
        strokes
    );
    let env = envelope(
        ids,
        ids.first_client_seq,
        mint,
        SyncOperationV1::DeleteNotebook {
            notebook_id: nb.notebook_id,
        },
    )?;
    Ok((plan_of(nb.notebook_id, vec![env]), desc))
}

/// Delete one page (0-based `page_index`) of a notebook. A notebook always keeps at least one
/// page, so deleting the last remaining page is refused (delete the notebook instead).
pub fn plan_delete_page(
    snap: &SyncSnapshotV1,
    sel: &str,
    page_index: usize,
    ids: &Ids,
    mint: &mut dyn FnMut() -> Uuid,
) -> Result<(Plan, String), String> {
    let (_, nb) = find_notebook(snap, sel)?;
    if nb.pages.len() < 2 {
        return Err(format!(
            "notebook {:?} has only one page; delete the notebook instead",
            nb.title
        ));
    }
    let page = nb.pages.get(page_index).ok_or_else(|| {
        format!(
            "notebook {:?} has {} pages; no page at index {page_index}",
            nb.title,
            nb.pages.len()
        )
    })?;
    let page_id: PageId = page.page_id;
    let desc = format!(
        "page {} of notebook {:?} ({}), {} strokes",
        page_index + 1,
        nb.title,
        page_id,
        page.strokes.len()
    );
    let env = envelope(
        ids,
        ids.first_client_seq,
        mint,
        SyncOperationV1::DeletePage {
            notebook_id: nb.notebook_id,
            page_id,
        },
    )?;
    Ok((plan_of(nb.notebook_id, vec![env]), desc))
}

/// Copy pages (0-based `pages`, in that order) of notebook `src` into notebook `dst`, inserted
/// immediately before `dst` page `before` (0-based), or at the end when `before` is `None`.
///
/// A copy is verbatim: each stroke keeps its style and every point (x, y, pressure); typed text
/// keeps its layout and content. Copies get fresh ids (each stroke its own lineage root), so the
/// source notebook is untouched and can be deleted separately. Page links are not copied. No
/// chrome-band check: this re-sends ink the Kindle already holds, not new content.
pub fn plan_copy_pages(
    snap: &SyncSnapshotV1,
    src: &str,
    pages: &[usize],
    dst: &str,
    before: Option<usize>,
    ids: &Ids,
    mint: &mut dyn FnMut() -> Uuid,
) -> Result<(Plan, String), String> {
    let (_, from) = find_notebook(snap, src)?;
    let (_, into) = find_notebook(snap, dst)?;
    if from.notebook_id == into.notebook_id {
        return Err("source and destination are the same notebook".into());
    }
    if pages.is_empty() {
        return Err("no pages to copy".into());
    }
    let mut after: Option<PageId> = match before {
        None => into.pages.last().map(|p| p.page_id),
        Some(0) => None,
        Some(i) => Some(
            into.pages
                .get(i - 1)
                .ok_or_else(|| {
                    format!(
                        "{:?} has {} pages; cannot insert before page {}",
                        into.title,
                        into.pages.len(),
                        i + 1
                    )
                })?
                .page_id,
        ),
    };
    let mut seq = ids.first_client_seq;
    let mut envelopes = Vec::new();
    let mut push = |op: SyncOperationV1, mint: &mut dyn FnMut() -> Uuid| -> Result<(), String> {
        envelopes.push(envelope(ids, seq, mint, op)?);
        seq += 1;
        Ok(())
    };
    let (mut strokes, mut texts) = (0usize, 0usize);
    let mut numbers = Vec::new();
    for &index in pages {
        let page = from.pages.get(index).ok_or_else(|| {
            format!(
                "{:?} has {} pages; no page {}",
                from.title,
                from.pages.len(),
                index + 1
            )
        })?;
        numbers.push((index + 1).to_string());
        let page_id = PageId::from_uuid(mint()).map_err(|e| e.to_string())?;
        push(
            SyncOperationV1::CreatePage {
                notebook_id: into.notebook_id,
                page: PageSpec {
                    page_id,
                    background: page.background.clone(),
                    render: page.render.clone(),
                },
                after_page_id: after,
            },
            mint,
        )?;
        after = Some(page_id);
        for stroke in &page.strokes {
            let id = StrokeId::from_uuid(mint()).map_err(|e| e.to_string())?;
            push(
                SyncOperationV1::AddStroke {
                    notebook_id: into.notebook_id,
                    page_id,
                    stroke: SyncStroke {
                        stroke_id: id,
                        root_id: id,
                        style: stroke.style.clone(),
                        points: stroke.points.clone(),
                    },
                },
                mint,
            )?;
            strokes += 1;
        }
        for text in &page.texts {
            let mut text = text.clone();
            text.text_id = TextId::from_uuid(mint()).map_err(|e| e.to_string())?;
            push(
                SyncOperationV1::AddText {
                    notebook_id: into.notebook_id,
                    page_id,
                    text,
                },
                mint,
            )?;
            texts += 1;
        }
    }
    let position = match before {
        None => "at the end".to_string(),
        Some(i) => format!("before page {}", i + 1),
    };
    let desc = format!(
        "copy pages {} of {:?} ({} strokes, {} texts) into {:?} {position}; {:?} itself is unchanged",
        numbers.join(", "),
        from.title,
        strokes,
        texts,
        into.title,
        from.title,
    );
    Ok((plan_of(into.notebook_id, envelopes), desc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sync_protocol::{
        BackgroundRef, PageSpace, PressureCurveRef, RenderProfileRef, SnapshotPage,
    };

    fn counter() -> impl FnMut() -> Uuid {
        let mut n: u64 = 0;
        move || {
            n += 1;
            let mut b = [0u8; 16];
            b[0] = 0x01;
            b[6] = 0x70;
            b[8] = 0x80;
            b[9..16].copy_from_slice(&n.to_be_bytes()[1..]);
            Uuid::from_bytes(b)
        }
    }

    fn snap(page_counts: &[usize]) -> SyncSnapshotV1 {
        let mut m = counter();
        let render = RenderProfileRef {
            profile_version: 2,
            ink_byte: 0,
            legacy_fixed_radius: 2,
            transform_id: "t".into(),
            rasterizer_id: "r".into(),
            pressure: Some(PressureCurveRef {
                q_r1_max: 1,
                q_r2_max: 2,
                q_r3_max: 3,
                min_radius: 1,
                max_radius: 4,
                missing_fallback_radius: 2,
            }),
        };
        SyncSnapshotV1 {
            v: 1,
            library_id: LibraryId::from_uuid(m()).unwrap(),
            page_space: PageSpace::CANONICAL,
            notebooks: page_counts
                .iter()
                .enumerate()
                .map(|(i, &n)| SnapshotNotebook {
                    notebook_id: NotebookId::from_uuid(m()).unwrap(),
                    title: format!("NB{i}"),
                    pages: (0..n)
                        .map(|_| SnapshotPage {
                            page_id: PageId::from_uuid(m()).unwrap(),
                            background: BackgroundRef::Blank,
                            render: render.clone(),
                            strokes: vec![],
                            texts: vec![],
                            links: vec![],
                        })
                        .collect(),
                    folder_id: None,
                })
                .collect(),
            folders: vec![],
        }
    }

    fn ids(s: &SyncSnapshotV1) -> Ids {
        let mut m = counter();
        for _ in 0..500 {
            m();
        }
        Ids {
            library_id: s.library_id,
            replica_id: ReplicaId::from_uuid(m()).unwrap(),
            first_client_seq: 5,
        }
    }

    #[test]
    fn selects_by_index_or_uuid() {
        let s = snap(&[1, 2]);
        assert_eq!(find_notebook(&s, "1").unwrap().1.title, "NB1");
        let id = s.notebooks[0].notebook_id.to_string();
        assert_eq!(find_notebook(&s, &id).unwrap().1.title, "NB0");
        assert!(find_notebook(&s, "7").is_err());
        assert!(find_notebook(&s, "not-an-id").is_err());
    }

    #[test]
    fn delete_notebook_builds_one_valid_envelope() {
        let s = snap(&[1, 3]);
        let (plan, desc) = plan_delete_notebook(&s, "1", &ids(&s), &mut counter()).unwrap();
        assert_eq!(plan.envelopes.len(), 1);
        assert_eq!(plan.envelopes[0].client_seq, 5);
        assert!(matches!(
            plan.envelopes[0].operation,
            SyncOperationV1::DeleteNotebook { notebook_id } if notebook_id == s.notebooks[1].notebook_id
        ));
        assert!(desc.contains("NB1") && desc.contains("3 pages"), "{desc}");
        plan.envelopes[0].to_canonical_bytes().unwrap();
    }

    #[test]
    fn delete_page_checks_bounds_and_last_page() {
        let s = snap(&[1, 3]);
        assert!(plan_delete_page(&s, "0", 0, &ids(&s), &mut counter()).is_err());
        assert!(plan_delete_page(&s, "1", 3, &ids(&s), &mut counter()).is_err());
        let (plan, desc) = plan_delete_page(&s, "1", 2, &ids(&s), &mut counter()).unwrap();
        assert!(matches!(
            &plan.envelopes[0].operation,
            SyncOperationV1::DeletePage { page_id, .. } if *page_id == s.notebooks[1].pages[2].page_id
        ));
        assert!(desc.starts_with("page 3 of notebook"), "{desc}");
    }

    #[test]
    fn list_formats_every_notebook() {
        let s = snap(&[1, 2]);
        let out = format_rows(&rows(&s));
        assert!(
            out.contains("[0]") && out.contains("[1]") && out.contains("pages=2"),
            "{out}"
        );
        assert_eq!(format_rows(&[]), "(no notebooks)\n");
    }
}
