//! Notebook folders: filing a new notebook in a folder, and moving existing notebooks between
//! folders, through the same operations the Kindle's folder UI emits (`CreateFolder`,
//! `SetNotebookFolder`).
//!
//! A folder is named by its title, matched case-insensitively after trimming (or by its full
//! id). **A folder that does not exist yet is created** (one `CreateFolder`, appended after the
//! existing folders, exactly like "New folder" on the Kindle); the report says so, so a caller
//! can show it before writing. Two live folders with the same title make the name ambiguous and
//! are refused.

use serde_json::{json, Value};
use sync_protocol::{
    validate_title, FolderId, NotebookId, SnapshotFolder, SyncOperationV1, SyncSnapshotV1,
};
use uuid::Uuid;

use crate::build::Plan;
use crate::manage::{envelope, find_notebook, Ids};

/// Where notebooks go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A live folder.
    Existing { id: FolderId, title: String },
    /// A folder this plan creates.
    New { id: FolderId, title: String },
    /// Back to the top level (no folder).
    TopLevel,
}

impl Target {
    pub fn id(&self) -> Option<FolderId> {
        match self {
            Self::Existing { id, .. } | Self::New { id, .. } => Some(*id),
            Self::TopLevel => None,
        }
    }

    pub fn title(&self) -> Option<&str> {
        match self {
            Self::Existing { title, .. } | Self::New { title, .. } => Some(title),
            Self::TopLevel => None,
        }
    }

    /// `{"id", "title", "created"}`, or `null` for the top level.
    pub fn report(&self) -> Value {
        match self {
            Self::TopLevel => Value::Null,
            Self::Existing { id, title } => {
                json!({"id": id.to_string(), "title": title, "created": false})
            }
            Self::New { id, title } => {
                json!({"id": id.to_string(), "title": title, "created": true})
            }
        }
    }

    /// `folder "Review"`, `new folder "Review"` or `the top level`.
    pub fn describe(&self) -> String {
        match self {
            Self::Existing { title, .. } => format!("folder {title:?}"),
            Self::New { title, .. } => format!("new folder {title:?} (created by this write)"),
            Self::TopLevel => "the top level".into(),
        }
    }
}

/// Resolve `name` against `folders` (live folders in library order). A full folder id must
/// name a live folder; anything else is a title.
pub fn resolve(
    folders: &[SnapshotFolder],
    name: &str,
    mint: &mut dyn FnMut() -> Uuid,
) -> Result<Target, String> {
    if let Ok(id) = FolderId::parse(name.trim()) {
        return folders
            .iter()
            .find(|f| f.folder_id == id)
            .map(|f| Target::Existing {
                id,
                title: f.title.clone(),
            })
            .ok_or_else(|| format!("no folder with id {id}"));
    }
    let title = name.trim();
    validate_title(title).map_err(|e| format!("folder name {name:?}: {e}"))?;
    let key = title.to_lowercase();
    let matches: Vec<&SnapshotFolder> = folders
        .iter()
        .filter(|f| f.title.trim().to_lowercase() == key)
        .collect();
    match matches.as_slice() {
        [] => Ok(Target::New {
            id: FolderId::from_uuid(mint()).map_err(|e| e.to_string())?,
            title: title.to_string(),
        }),
        [f] => Ok(Target::Existing {
            id: f.folder_id,
            title: f.title.clone(),
        }),
        many => Err(format!(
            "{} folders are called {title:?} ({}); give the folder id instead",
            many.len(),
            many.iter()
                .map(|f| f.folder_id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// The operations that put `notebooks` in `target`, starting at `first_seq`: `CreateFolder`
/// (new folders only), then one `SetNotebookFolder` per notebook.
pub fn operations(target: &Target, notebooks: &[NotebookId]) -> Vec<SyncOperationV1> {
    let mut ops = Vec::new();
    if let Target::New { id, title } = target {
        ops.push(SyncOperationV1::CreateFolder {
            folder_id: *id,
            title: title.clone(),
        });
    }
    for &notebook_id in notebooks {
        ops.push(SyncOperationV1::SetNotebookFolder {
            notebook_id,
            folder_id: target.id(),
        });
    }
    ops
}

/// Append the folder operations for a newly built notebook to its plan (after its content, so
/// an interrupted write leaves a complete notebook at the top level rather than a half one in
/// the folder).
pub fn file_new_notebook(
    plan: &mut Plan,
    target: &Target,
    ids: &Ids,
    mint: &mut dyn FnMut() -> Uuid,
) -> Result<(), String> {
    let mut seq = plan
        .envelopes
        .last()
        .map(|e| e.client_seq + 1)
        .unwrap_or(ids.first_client_seq);
    for op in operations(target, &[plan.notebook_id]) {
        plan.envelopes.push(envelope(ids, seq, mint, op)?);
        seq += 1;
    }
    Ok(())
}

/// One notebook a move plan covers.
pub struct Moved {
    pub id: NotebookId,
    pub title: String,
    /// Folder title before the move (`None` = top level).
    pub from: Option<String>,
}

/// Move the notebooks `sels` (list index or full id each) into `target`. Notebooks already
/// there are left out of the plan (and listed in `unchanged`); if that is all of them, the move
/// is refused as a no-op.
pub fn plan_move(
    snap: &SyncSnapshotV1,
    sels: &[String],
    target: &Target,
    ids: &Ids,
    mint: &mut dyn FnMut() -> Uuid,
) -> Result<(Plan, Vec<Moved>, Vec<Moved>, String), String> {
    if sels.is_empty() {
        return Err("give at least one notebook (list index or full id)".into());
    }
    let folder_title = |f: Option<FolderId>| {
        f.and_then(|id| {
            snap.folders
                .iter()
                .find(|x| x.folder_id == id)
                .map(|x| x.title.clone())
        })
    };
    let (mut moved, mut unchanged) = (Vec::new(), Vec::new());
    for sel in sels {
        let (_, nb) = find_notebook(snap, sel)?;
        if moved
            .iter()
            .chain(unchanged.iter())
            .any(|m: &Moved| m.id == nb.notebook_id)
        {
            continue;
        }
        let m = Moved {
            id: nb.notebook_id,
            title: nb.title.clone(),
            from: folder_title(nb.folder_id),
        };
        if nb.folder_id == target.id() {
            unchanged.push(m);
        } else {
            moved.push(m);
        }
    }
    if moved.is_empty() {
        return Err(format!(
            "nothing to move: {} already in {}",
            unchanged
                .iter()
                .map(|m| format!("{:?}", m.title))
                .collect::<Vec<_>>()
                .join(", "),
            target.describe()
        ));
    }
    let mut envelopes = Vec::new();
    let notebook_ids: Vec<NotebookId> = moved.iter().map(|m| m.id).collect();
    for (k, op) in operations(target, &notebook_ids).into_iter().enumerate() {
        envelopes.push(envelope(ids, ids.first_client_seq + k as u64, mint, op)?);
    }
    let list = moved
        .iter()
        .map(|m| match &m.from {
            Some(f) => format!("{:?} (from folder {f:?})", m.title),
            None => format!("{:?}", m.title),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut desc = format!("move {list} into {}", target.describe());
    if !unchanged.is_empty() {
        desc.push_str(&format!(
            "; already there: {}",
            unchanged
                .iter()
                .map(|m| format!("{:?}", m.title))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let plan = Plan {
        notebook_id: moved[0].id,
        envelopes,
        blobs: Vec::new(),
        pages: Vec::new(),
    };
    Ok((plan, moved, unchanged, desc))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn folders() -> Vec<SnapshotFolder> {
        let mut m = counter();
        vec![SnapshotFolder {
            folder_id: FolderId::from_uuid(m()).unwrap(),
            title: "Weekly".into(),
        }]
    }

    #[test]
    fn resolves_existing_folders_case_insensitively_and_creates_missing_ones() {
        let f = folders();
        let mut m = counter();
        for _ in 0..100 {
            m();
        }
        assert_eq!(
            resolve(&f, " weekly ", &mut m).unwrap(),
            Target::Existing {
                id: f[0].folder_id,
                title: "Weekly".into()
            }
        );
        assert_eq!(
            resolve(&f, &f[0].folder_id.to_string(), &mut m)
                .unwrap()
                .title(),
            Some("Weekly")
        );
        match resolve(&f, " Review ", &mut m).unwrap() {
            Target::New { title, .. } => assert_eq!(title, "Review"),
            t => panic!("{t:?}"),
        }
        assert!(resolve(&f, "   ", &mut m).is_err());
        assert!(resolve(&f, &"x".repeat(97), &mut m).is_err());
    }

    #[test]
    fn spec_takes_an_optional_folder() {
        let s = crate::spec::Spec::parse(
            br#"{"notebook":{"title":"T","folder":"Weekly"},"pages":[{}]}"#,
        )
        .unwrap();
        assert_eq!(s.notebook.folder.as_deref(), Some("Weekly"));
        let s = crate::spec::Spec::parse(br#"{"notebook":{"title":"T"},"pages":[{}]}"#).unwrap();
        assert_eq!(s.notebook.folder, None);
        assert!(crate::spec::Spec::parse(
            br#"{"notebook":{"title":"T","shelf":"x"},"pages":[{}]}"#
        )
        .is_err());
    }

    #[test]
    fn duplicate_titles_are_ambiguous() {
        let mut f = folders();
        let mut m = counter();
        for _ in 0..50 {
            m();
        }
        f.push(SnapshotFolder {
            folder_id: FolderId::from_uuid(m()).unwrap(),
            title: "WEEKLY".into(),
        });
        let e = resolve(&f, "Weekly", &mut m).unwrap_err();
        assert!(e.contains("2 folders"), "{e}");
    }

    #[test]
    fn operations_create_the_folder_first() {
        let mut m = counter();
        let id = FolderId::from_uuid(m()).unwrap();
        let nb = NotebookId::from_uuid(m()).unwrap();
        let ops = operations(
            &Target::New {
                id,
                title: "Review".into(),
            },
            &[nb],
        );
        assert!(
            matches!(&ops[0], SyncOperationV1::CreateFolder { folder_id, title } if *folder_id == id && title == "Review")
        );
        assert!(
            matches!(&ops[1], SyncOperationV1::SetNotebookFolder { notebook_id, folder_id: Some(f) } if *notebook_id == nb && *f == id)
        );
        let ops = operations(&Target::TopLevel, &[nb]);
        assert_eq!(ops.len(), 1);
        assert!(matches!(
            &ops[0],
            SyncOperationV1::SetNotebookFolder {
                folder_id: None,
                ..
            }
        ));
    }
}
