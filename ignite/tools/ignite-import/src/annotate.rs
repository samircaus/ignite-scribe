//! `annotate`: add ink and typed text to an **existing** page, or to a new page right after it.
//!
//! Only additive operations are emitted (`AddStroke`, `AddText`, and with `new_page`:
//! `CreatePage` + `AddPageLink`), each stroke its own lineage root, all authored by the
//! importer's replica, so the page's own ink is never touched and the additions can later be
//! removed again by id ([`plan_delete_ink`]). The input is one page of the ordinary page spec
//! (`shapes`, `strokes`, `labels`, `writing`, `drawings`, `texts`) and goes through the same
//! builder and checks as a new notebook (chrome bands, page bounds, envelope validation).

use serde::Deserialize;
use sync_protocol::{
    LinkId, PageId, PageSpec, StrokeId, SyncOperationV1, SyncPageLink, SyncSnapshotV1, TextId,
    CANONICAL_PAGE_WIDTH,
};
use uuid::Uuid;

use crate::build::{build, safe_y_range, Ctx, Plan};
use crate::manage::{envelope, find_notebook, Ids};
use crate::spec::{
    Coords, DrawingIn, Label, NotebookIn, PageIn, Shape, Spec, StrokeIn, TextIn, WritingIn,
};

/// One page of additions, in the page-spec format.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AnnotateSpec {
    #[serde(default)]
    pub coords: Coords,
    #[serde(default)]
    pub shapes: Vec<Shape>,
    #[serde(default)]
    pub strokes: Vec<StrokeIn>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub writing: Vec<WritingIn>,
    #[serde(default)]
    pub drawings: Vec<DrawingIn>,
    #[serde(default)]
    pub texts: Vec<TextIn>,
    /// Put the additions on a new page inserted right after the target page.
    #[serde(default)]
    pub new_page: Option<NewPageIn>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NewPageIn {
    /// Where on the target page to place a link icon to the new page (spec coordinates). The new
    /// page then also gets a link back in its top-right corner.
    #[serde(default)]
    pub link_from: Option<[f32; 2]>,
}

impl AnnotateSpec {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let s: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("invalid annotate spec: {e}"))?;
        if s.shapes.is_empty()
            && s.strokes.is_empty()
            && s.labels.is_empty()
            && s.writing.is_empty()
            && s.drawings.is_empty()
            && s.texts.is_empty()
        {
            return Err("invalid annotate spec: nothing to add".into());
        }
        Ok(s)
    }
}

/// What an annotate plan adds (for the JSON report and the undo journal).
#[derive(Clone, Debug, PartialEq)]
pub struct Added {
    pub notebook_id: String,
    pub notebook_title: String,
    /// The page that holds the additions (the new page when `new_page` was used).
    pub page_id: String,
    /// The page that was asked for.
    pub target_page_id: String,
    pub created_page_id: Option<String>,
    pub stroke_ids: Vec<String>,
    pub text_ids: Vec<String>,
    pub link_ids: Vec<String>,
}

/// Plan the additions to page `page_index` (0-based) of notebook `sel` (list index or id).
pub fn plan_annotate(
    snap: &SyncSnapshotV1,
    sel: &str,
    page_index: usize,
    add: &AnnotateSpec,
    ids: &Ids,
    mint: &mut dyn FnMut() -> Uuid,
) -> Result<(Plan, Added, String), String> {
    let (_, nb) = find_notebook(snap, sel)?;
    let page = nb.pages.get(page_index).ok_or_else(|| {
        format!(
            "notebook {:?} has {} pages; no page {}",
            nb.title,
            nb.pages.len(),
            page_index + 1
        )
    })?;

    // Build as if it were a one-page notebook, then keep only the page content, retargeted.
    let spec = Spec {
        coords: add.coords.clone(),
        notebook: NotebookIn {
            title: nb.title.clone(),
            folder: None,
        },
        pages: vec![PageIn {
            shapes: add.shapes.clone(),
            strokes: add.strokes.clone(),
            labels: add.labels.clone(),
            writing: add.writing.clone(),
            drawings: add.drawings.clone(),
            texts: add.texts.clone(),
            ..PageIn::default()
        }],
    };
    let ctx = Ctx {
        library_id: ids.library_id,
        replica_id: ids.replica_id,
        first_client_seq: ids.first_client_seq,
        render: page.render.clone(),
        after_notebook_id: None,
    };
    let built = build(&spec, &ctx, mint)?;
    let unit = match add.coords {
        Coords::PageWidth => CANONICAL_PAGE_WIDTH,
        Coords::Canonical => 1.0,
    };

    let mut envelopes = Vec::new();
    let mut seq = ids.first_client_seq;
    let mut push = |op: SyncOperationV1, mint: &mut dyn FnMut() -> Uuid| -> Result<(), String> {
        envelopes.push(envelope(ids, seq, mint, op)?);
        seq += 1;
        Ok(())
    };

    let mut link_ids = Vec::new();
    let dest: PageId = match &add.new_page {
        None => page.page_id,
        Some(np) => {
            let new_id = PageId::from_uuid(mint()).map_err(|e| e.to_string())?;
            push(
                SyncOperationV1::CreatePage {
                    notebook_id: nb.notebook_id,
                    page: PageSpec {
                        page_id: new_id,
                        background: page.background.clone(),
                        render: page.render.clone(),
                    },
                    after_page_id: Some(page.page_id),
                },
                mint,
            )?;
            if let Some([x, y]) = np.link_from {
                let (top, _) = safe_y_range();
                let links = [
                    (page.page_id, x * unit, y * unit, new_id),
                    (
                        new_id,
                        CANONICAL_PAGE_WIDTH * 0.94,
                        top + 0.03 * CANONICAL_PAGE_WIDTH,
                        page.page_id,
                    ),
                ];
                for (from, sx, sy, to) in links {
                    let link_id = LinkId::from_uuid(mint()).map_err(|e| e.to_string())?;
                    link_ids.push(link_id.to_string());
                    push(
                        SyncOperationV1::AddPageLink {
                            notebook_id: nb.notebook_id,
                            page_id: from,
                            link: SyncPageLink {
                                link_id,
                                source_notebook_id: nb.notebook_id,
                                source_page_id: from,
                                source_x: sx,
                                source_y: sy,
                                target_notebook_id: nb.notebook_id,
                                target_page_id: to,
                            },
                        },
                        mint,
                    )
                    .map_err(|e| format!("page link: {e}"))?;
                }
            }
            new_id
        }
    };

    let (mut stroke_ids, mut text_ids) = (Vec::new(), Vec::new());
    for env in built.envelopes {
        let op = match env.operation {
            SyncOperationV1::AddStroke { stroke, .. } => {
                stroke_ids.push(stroke.stroke_id.to_string());
                SyncOperationV1::AddStroke {
                    notebook_id: nb.notebook_id,
                    page_id: dest,
                    stroke,
                }
            }
            SyncOperationV1::AddText { text, .. } => {
                text_ids.push(text.text_id.to_string());
                SyncOperationV1::AddText {
                    notebook_id: nb.notebook_id,
                    page_id: dest,
                    text,
                }
            }
            SyncOperationV1::CreateNotebook { .. } | SyncOperationV1::CreatePage { .. } => continue,
            other => return Err(format!("annotate: unexpected operation {other:?}")),
        };
        push(op, mint)?;
    }

    let mut preview = built
        .pages
        .into_iter()
        .next()
        .unwrap_or_else(|| unreachable!());
    preview.paper = crate::spec::Paper::Blank;
    let place = match &add.new_page {
        None => format!("page {} of {:?}", page_index + 1, nb.title),
        Some(_) => format!(
            "a new page inserted after page {} of {:?}{}",
            page_index + 1,
            nb.title,
            if link_ids.is_empty() {
                ""
            } else {
                ", linked both ways"
            }
        ),
    };
    let desc = format!(
        "add {} strokes and {} text objects to {place}; nothing already there is changed",
        stroke_ids.len(),
        text_ids.len(),
    );
    let plan = Plan {
        notebook_id: nb.notebook_id,
        envelopes,
        blobs: Vec::new(),
        pages: vec![preview],
        warnings: built.warnings,
    };
    let added = Added {
        notebook_id: nb.notebook_id.to_string(),
        notebook_title: nb.title.clone(),
        page_id: dest.to_string(),
        target_page_id: page.page_id.to_string(),
        created_page_id: add.new_page.as_ref().map(|_| dest.to_string()),
        stroke_ids,
        text_ids,
        link_ids,
    };
    Ok((plan, added, desc))
}

// ---------------------------------------------------------------- undo

/// One line of the importer's ink journal (`<state-dir>/ai-ink.jsonl`): what one `annotate`
/// wrote. Undo only ever removes ids that are listed here, i.e. ink this replica created.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, Deserialize)]
pub struct JournalEntry {
    pub at: String,
    pub notebook_id: String,
    pub page_id: String,
    #[serde(default)]
    pub created_page_id: Option<String>,
    #[serde(default)]
    pub stroke_ids: Vec<String>,
    #[serde(default)]
    pub text_ids: Vec<String>,
    #[serde(default)]
    pub link_ids: Vec<String>,
    pub head_server_seq: i64,
}

pub const JOURNAL_FILE: &str = "ai-ink.jsonl";

pub fn read_journal(dir: &std::path::Path) -> Result<Vec<JournalEntry>, String> {
    let path = dir.join(JOURNAL_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| {
            serde_json::from_str(l).map_err(|e| format!("{} line {}: {e}", path.display(), i + 1))
        })
        .collect()
}

pub fn append_journal(dir: &std::path::Path, entry: &JournalEntry) -> Result<(), String> {
    use std::io::Write;
    let path = dir.join(JOURNAL_FILE);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    writeln!(f, "{line}").map_err(|e| format!("{}: {e}", path.display()))
}

/// What undo will remove, and what it skips.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Removal {
    pub strokes: usize,
    pub texts: usize,
    /// Ids asked for that are already gone (erased on the Kindle, or undone before).
    pub already_gone: usize,
}

/// Remove ink this replica wrote. `ids` are stroke (root) or text ids; each must appear in the
/// journal, otherwise the whole request is refused, so a human stroke can never be deleted.
/// Annotate strokes that were partly erased (and so split) go entirely: `DeleteStrokeLineage` removes every fragment.
pub fn plan_delete_ink(
    snap: &SyncSnapshotV1,
    journal: &[JournalEntry],
    ids_wanted: &[String],
    ids: &Ids,
    mint: &mut dyn FnMut() -> Uuid,
) -> Result<(Plan, Removal, String), String> {
    if ids_wanted.is_empty() {
        return Err("nothing to remove: give stroke or text ids".into());
    }
    let mut envelopes = Vec::new();
    let mut seq = ids.first_client_seq;
    let mut removal = Removal::default();
    let mut notebook = None;
    let mut fallback = None;
    // Check every id first, so a mixed request is refused before anything is planned.
    for want in ids_wanted {
        if !journal
            .iter()
            .any(|e| e.stroke_ids.contains(want) || e.text_ids.contains(want))
        {
            return Err(format!(
                "{want} was not written by this importer (not in its journal); refusing"
            ));
        }
    }
    for want in ids_wanted {
        let entry = journal
            .iter()
            .find(|e| e.stroke_ids.contains(want) || e.text_ids.contains(want))
            .unwrap_or_else(|| unreachable!());
        if fallback.is_none() {
            fallback = sync_protocol::NotebookId::parse(&entry.notebook_id).ok();
        }
        let nb = snap
            .notebooks
            .iter()
            .find(|n| n.notebook_id.to_string() == entry.notebook_id);
        let Some(nb) = nb else {
            removal.already_gone += 1;
            continue;
        };
        let Some(page) = nb
            .pages
            .iter()
            .find(|p| p.page_id.to_string() == entry.page_id)
        else {
            removal.already_gone += 1;
            continue;
        };
        notebook.get_or_insert(nb.notebook_id);
        let op = if entry.stroke_ids.contains(want) {
            let root = StrokeId::parse(want).map_err(|e| e.to_string())?;
            if !page.strokes.iter().any(|s| s.root_id == root) {
                removal.already_gone += 1;
                continue;
            }
            removal.strokes += 1;
            SyncOperationV1::DeleteStrokeLineage {
                notebook_id: nb.notebook_id,
                page_id: page.page_id,
                root_id: root,
            }
        } else {
            let text_id = TextId::parse(want).map_err(|e| e.to_string())?;
            if !page.texts.iter().any(|t| t.text_id == text_id) {
                removal.already_gone += 1;
                continue;
            }
            removal.texts += 1;
            SyncOperationV1::DeleteText {
                notebook_id: nb.notebook_id,
                page_id: page.page_id,
                text_id,
            }
        };
        envelopes.push(envelope(ids, seq, mint, op)?);
        seq += 1;
    }
    let desc = format!(
        "remove {} added strokes and {} added text objects ({} already gone); your own ink is untouched",
        removal.strokes, removal.texts, removal.already_gone
    );
    let plan = Plan {
        notebook_id: notebook
            .or(fallback)
            .ok_or("the journal entry has no valid notebook id")?,
        envelopes,
        blobs: Vec::new(),
        pages: Vec::new(),
        warnings: Vec::new(),
    };
    Ok((plan, removal, desc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sync_protocol::{
        BackgroundRef, LibraryId, NotebookId, PageSpace, RenderProfileRef, ReplicaId,
        SnapshotNotebook, SnapshotPage, StrokeStyle, SyncPoint, SyncStroke,
    };

    fn counter(start: u64) -> impl FnMut() -> Uuid {
        let mut n = start;
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

    fn snap() -> SyncSnapshotV1 {
        let mut m = counter(0);
        let render = crate::build::default_render();
        let page = |m: &mut dyn FnMut() -> Uuid, render: &RenderProfileRef| SnapshotPage {
            page_id: PageId::from_uuid(m()).unwrap(),
            background: BackgroundRef::Blank,
            render: render.clone(),
            strokes: vec![],
            texts: vec![],
            links: vec![],
        };
        SyncSnapshotV1 {
            v: 1,
            library_id: LibraryId::from_uuid(m()).unwrap(),
            page_space: PageSpace::CANONICAL,
            notebooks: vec![SnapshotNotebook {
                notebook_id: NotebookId::from_uuid(m()).unwrap(),
                title: "Ideas".into(),
                pages: vec![page(&mut m, &render), page(&mut m, &render)],
                folder_id: None,
            }],
            folders: vec![],
        }
    }

    fn ids(s: &SyncSnapshotV1) -> Ids {
        Ids {
            library_id: s.library_id,
            replica_id: ReplicaId::from_uuid(counter(900)()).unwrap(),
            first_client_seq: 7,
        }
    }

    fn spec() -> AnnotateSpec {
        AnnotateSpec::parse(
            br#"{ "labels": [ { "text": "OK", "x": 0.6, "y": 0.3, "size": 0.03 } ],
                  "shapes": [ { "kind": "arrow", "from": [0.5, 0.31], "to": [0.58, 0.31] } ],
                  "writing": [ { "text": "three directions:\nfirst, second", "x": 0.1, "y": 0.5,
                                 "size": 0.025, "width": 0.5 } ],
                  "drawings": [ { "svg": "<svg viewBox='0 0 10 10'><circle cx='5' cy='5' r='4'/></svg>",
                                  "x": 0.6, "y": 0.6, "width": 0.1 } ],
                  "texts": [ { "x": 0.1, "y": 0.9, "width": 0.8, "font_size": 0.02,
                               "content": "a longer typed passage" } ] }"#,
        )
        .unwrap()
    }

    #[test]
    fn adds_only_strokes_and_texts_to_the_chosen_page_with_dense_seqs() {
        let s = snap();
        let (plan, added, _) =
            plan_annotate(&s, "0", 1, &spec(), &ids(&s), &mut counter(2000)).unwrap();
        let target = s.notebooks[0].pages[1].page_id;
        assert_eq!(added.text_ids.len(), 1);
        assert!(
            added.stroke_ids.len() > 20,
            "writing and drawing add strokes"
        );
        assert_eq!(added.stroke_ids.len() + 1, plan.envelopes.len());
        assert_eq!(added.created_page_id, None);
        for (i, e) in plan.envelopes.iter().enumerate() {
            assert_eq!(e.client_seq, 7 + i as u64);
            match &e.operation {
                SyncOperationV1::AddStroke {
                    notebook_id,
                    page_id,
                    stroke,
                } => {
                    assert_eq!(
                        (*notebook_id, *page_id),
                        (s.notebooks[0].notebook_id, target)
                    );
                    assert_eq!(stroke.stroke_id, stroke.root_id);
                }
                SyncOperationV1::AddText {
                    notebook_id,
                    page_id,
                    ..
                } => {
                    assert_eq!(
                        (*notebook_id, *page_id),
                        (s.notebooks[0].notebook_id, target)
                    );
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert!(plan.blobs.is_empty());
    }

    #[test]
    fn new_page_goes_right_after_the_target_with_links_both_ways() {
        let s = snap();
        let mut add = spec();
        add.new_page = Some(NewPageIn {
            link_from: Some([0.9, 0.4]),
        });
        let (plan, added, desc) =
            plan_annotate(&s, "0", 0, &add, &ids(&s), &mut counter(2000)).unwrap();
        let first = s.notebooks[0].pages[0].page_id;
        let SyncOperationV1::CreatePage {
            page,
            after_page_id,
            ..
        } = &plan.envelopes[0].operation
        else {
            panic!("first op must create the page")
        };
        assert_eq!(*after_page_id, Some(first));
        assert_eq!(
            added.created_page_id.as_deref(),
            Some(page.page_id.to_string().as_str())
        );
        assert_eq!(added.link_ids.len(), 2);
        let links: Vec<_> = plan
            .envelopes
            .iter()
            .filter_map(|e| match &e.operation {
                SyncOperationV1::AddPageLink { link, .. } => Some(link),
                _ => None,
            })
            .collect();
        assert_eq!(
            (links[0].source_page_id, links[0].target_page_id),
            (first, page.page_id)
        );
        assert_eq!(
            (links[1].source_page_id, links[1].target_page_id),
            (page.page_id, first)
        );
        assert!((links[0].source_x - 0.9 * CANONICAL_PAGE_WIDTH).abs() < 1e-2);
        for e in &plan.envelopes {
            if let SyncOperationV1::AddStroke { page_id, .. } = &e.operation {
                assert_eq!(*page_id, page.page_id);
            }
        }
        assert!(desc.contains("new page"), "{desc}");
    }

    #[test]
    fn refuses_missing_pages_empty_specs_and_chrome_bands() {
        let s = snap();
        assert!(plan_annotate(&s, "0", 2, &spec(), &ids(&s), &mut counter(2000)).is_err());
        assert!(AnnotateSpec::parse(b"{}").is_err());
        assert!(AnnotateSpec::parse(br#"{"paper":"lined","strokes":[]}"#).is_err());
        let top =
            AnnotateSpec::parse(br#"{"strokes":[{"points":[[0.1,0.001],[0.2,0.001]]}]}"#).unwrap();
        assert!(plan_annotate(&s, "0", 0, &top, &ids(&s), &mut counter(2000)).is_err());
        let bad_svg = AnnotateSpec::parse(
            br#"{"drawings":[{"svg":"<svg><path d='Q'/></svg>","x":0.1,"y":0.5,"width":0.2}]}"#,
        )
        .unwrap();
        assert!(plan_annotate(&s, "0", 0, &bad_svg, &ids(&s), &mut counter(2000)).is_err());
    }

    fn with_strokes(mut s: SyncSnapshotV1, roots: &[&str]) -> SyncSnapshotV1 {
        for r in roots {
            let id = StrokeId::parse(r).unwrap();
            s.notebooks[0].pages[0].strokes.push(SyncStroke {
                stroke_id: id,
                root_id: id,
                style: StrokeStyle::LegacyFixedV1,
                points: vec![
                    SyncPoint::new(100., 3000., None),
                    SyncPoint::new(200., 3000., None),
                ],
            });
        }
        s
    }

    #[test]
    fn undo_removes_only_journaled_ink_and_counts_what_is_gone() {
        let mine = "01a1df4c-7da4-714f-b4f1-d209d162af6b";
        let erased = "01a1df4c-7da4-714f-b4f1-d20b50a79b93";
        let human = "01a1b17d-d788-72e6-acef-608133e490b0";
        let s = with_strokes(snap(), &[mine, human]);
        let journal = vec![JournalEntry {
            notebook_id: s.notebooks[0].notebook_id.to_string(),
            page_id: s.notebooks[0].pages[0].page_id.to_string(),
            stroke_ids: vec![mine.into(), erased.into()],
            ..Default::default()
        }];
        let (plan, removal, _) = plan_delete_ink(
            &s,
            &journal,
            &[mine.into(), erased.into()],
            &ids(&s),
            &mut counter(3000),
        )
        .unwrap();
        assert_eq!(
            removal,
            Removal {
                strokes: 1,
                texts: 0,
                already_gone: 1
            }
        );
        assert_eq!(plan.envelopes.len(), 1);
        assert!(matches!(
            &plan.envelopes[0].operation,
            SyncOperationV1::DeleteStrokeLineage { root_id, .. } if root_id.to_string() == mine
        ));
        let e = plan_delete_ink(
            &s,
            &journal,
            &[mine.into(), human.into()],
            &ids(&s),
            &mut counter(3000),
        )
        .err()
        .unwrap();
        assert!(e.contains(human) && e.contains("refusing"), "{e}");
    }

    #[test]
    fn journal_round_trips() {
        let dir =
            std::env::temp_dir().join(format!("ignite-import-journal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_journal(&dir).unwrap().is_empty());
        let e = JournalEntry {
            at: "2026-11-15T14:00:00Z".into(),
            stroke_ids: vec!["a".into()],
            head_server_seq: 9,
            ..Default::default()
        };
        append_journal(&dir, &e).unwrap();
        append_journal(&dir, &e).unwrap();
        assert_eq!(read_journal(&dir).unwrap(), vec![e.clone(), e]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
