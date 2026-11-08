//! Gate F11 modal dialogs: notebook card menu, page menu, confirmations, notices, errors, and
//! the title-entry keyboard. Modals own input while open; nothing underneath sees a tap.

use kindle_platform::FbGeometry;

use crate::keyboard::TitleEditor;
use crate::ui_gfx::{sans_wrap, Canvas, Rect, UiPoint, BLACK, GRAY, WHITE};

/// Dialog sheet: at most this wide, centred, with this much side margin on narrow panels.
const SHEET_MAX_W: u32 = 1180;
const SHEET_MARGIN_X: u32 = 100;
const PAD: u32 = 56;
const BORDER: u32 = 4;
const RADIUS: f32 = 34.0;
/// Offset of the flat "shadow" under the sheet.
const SHADOW: u32 = 12;
const HEADING_PX: f32 = 52.0;
const HEADING_LINE: u32 = 68;
const BODY_PX: f32 = 38.0;
const BODY_LINE: u32 = 52;
const LABEL_PX: f32 = 42.0;
/// Menu rows (vertical dialogs) and pill buttons (horizontal dialogs).
const ROW_H: u32 = 136;
const PILL_H: u32 = 132;
const BUTTON_GAP: u32 = 28;
const ICON_BOX: u32 = 96;
const DARK: u8 = 0x50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TitlePurpose {
    Create,
    Rename {
        notebook_id: String,
        original: String,
    },
    /// D1: manual Scribe Server address (Sync screen fallback).
    ServerUrl,
    /// Kindle-local folders (`folders.rs`).
    CreateFolder,
    RenameFolder {
        folder_id: String,
        original: String,
    },
    /// + NEW inside an open folder: the new notebook goes into that folder.
    CreateNotebookInFolder {
        folder_id: String,
    },
}

/// Folder buttons shown per picker page (PREVIOUS/NEXT page through the rest).
pub const PICKER_PAGE: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Modal {
    /// Notebook menu on the main list (the notebook is in no folder).
    CardMenu {
        notebook_id: String,
        title: String,
    },
    /// Notebook menu for a notebook inside `folder_id`.
    CardMenuInFolder {
        notebook_id: String,
        title: String,
        folder_id: String,
    },
    /// + NEW on the main list: notebook or folder.
    NewChooser {
        notebook_title: String,
        folder_title: String,
    },
    FolderMenu {
        folder_id: String,
        title: String,
    },
    ConfirmDeleteFolder {
        folder_id: String,
        title: String,
    },
    /// Choose the folder for `notebook_id`: `options` are `(folder id, title)`.
    FolderPicker {
        notebook_id: String,
        title: String,
        options: Vec<(String, String)>,
        page: usize,
    },
    ConfirmDeleteNotebook {
        notebook_id: String,
        title: String,
    },
    TitleEntry {
        purpose: TitlePurpose,
        editor: TitleEditor,
    },
    PageMenu {
        page_number: usize,
        page_count: usize,
    },
    ConfirmDeletePage {
        page_number: usize,
    },
    /// Informational refusal (e.g. sole notebook). OK closes.
    Notice {
        heading: String,
        body: Vec<String>,
    },
    /// Jump / remove an explicit page link. Outside tap dismisses (no ink).
    LinkJump {
        link_id: sync_protocol::LinkId,
        /// Primary button label (`Jump to "Title - Page N"` or the missing-dest copy).
        label: String,
        target_live: bool,
        notebook_local: Option<String>,
        page_local: Option<String>,
    },
    /// "Something went wrong". OK closes, or returns to `resume` (e.g. the title editor).
    Error {
        message: String,
        resume: Option<Box<Modal>>,
    },
}

impl Modal {
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
            resume: None,
        }
    }

    pub fn sole_notebook_notice() -> Self {
        Self::Notice {
            heading: "Can't delete".into(),
            body: vec![
                "You can't delete the only notebook.".into(),
                "Create another notebook first.".into(),
            ],
        }
    }

    pub fn sole_page_notice() -> Self {
        Self::Notice {
            heading: "Can't delete".into(),
            body: vec![
                "You can't delete the only page.".into(),
                "Insert another page first.".into(),
            ],
        }
    }

    pub fn no_folders_notice() -> Self {
        Self::Notice {
            heading: "No folders yet".into(),
            body: vec!["Create one with + NEW, then NEW FOLDER.".into()],
        }
    }

    pub fn no_other_folder_notice() -> Self {
        Self::Notice {
            heading: "No other folder".into(),
            body: vec!["Create another folder with + NEW, then NEW FOLDER.".into()],
        }
    }

    pub fn is_title_entry(&self) -> bool {
        matches!(self, Self::TitleEntry { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonId {
    Rename,
    DeleteNotebook,
    InsertPageAfter,
    MovePageLeft,
    MovePageRight,
    DeletePage,
    ConfirmDelete,
    Cancel,
    Ok,
    NewNotebook,
    NewFolder,
    AddToFolder,
    ChangeFolder,
    RemoveFromFolder,
    DeleteFolder,
    /// Index into the picker's `options`.
    PickFolder(usize),
    PickerPrevious,
    PickerNext,
    Jump,
    RemoveLink,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogButton {
    pub id: ButtonId,
    pub label: String,
    pub enabled: bool,
    pub emphasized: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogSpec {
    pub heading: String,
    pub body: Vec<String>,
    pub buttons: Vec<DialogButton>,
    /// Menus stack buttons vertically; confirmations use one horizontal row.
    pub vertical: bool,
}

fn button(id: ButtonId, label: &str) -> DialogButton {
    DialogButton {
        id,
        label: label.into(),
        enabled: true,
        emphasized: false,
    }
}

/// Dialog content for every modal except the full-screen title editor.
pub fn dialog_spec(modal: &Modal) -> Option<DialogSpec> {
    Some(match modal {
        Modal::CardMenu { title, .. } => DialogSpec {
            heading: title.clone(),
            body: Vec::new(),
            buttons: vec![
                button(ButtonId::Rename, "Rename"),
                button(ButtonId::AddToFolder, "Add to folder"),
                button(ButtonId::DeleteNotebook, "Delete"),
                button(ButtonId::Cancel, "Cancel"),
            ],
            vertical: true,
        },
        Modal::CardMenuInFolder { title, .. } => DialogSpec {
            heading: title.clone(),
            body: Vec::new(),
            buttons: vec![
                button(ButtonId::Rename, "Rename"),
                button(ButtonId::ChangeFolder, "Change folder"),
                button(ButtonId::RemoveFromFolder, "Remove from folder"),
                button(ButtonId::DeleteNotebook, "Delete"),
                button(ButtonId::Cancel, "Cancel"),
            ],
            vertical: true,
        },
        Modal::NewChooser { .. } => DialogSpec {
            heading: "Create new".into(),
            body: Vec::new(),
            buttons: vec![
                button(ButtonId::NewNotebook, "New notebook"),
                button(ButtonId::NewFolder, "New folder"),
                button(ButtonId::Cancel, "Cancel"),
            ],
            vertical: true,
        },
        Modal::FolderMenu { title, .. } => DialogSpec {
            heading: title.clone(),
            body: Vec::new(),
            buttons: vec![
                button(ButtonId::Rename, "Rename"),
                button(ButtonId::DeleteFolder, "Delete"),
                button(ButtonId::Cancel, "Cancel"),
            ],
            vertical: true,
        },
        Modal::ConfirmDeleteFolder { title, .. } => DialogSpec {
            heading: format!("Delete folder \"{title}\"?"),
            body: vec!["Its notebooks move back to the main list. No notebook is deleted.".into()],
            buttons: vec![
                button(ButtonId::Cancel, "Cancel"),
                DialogButton {
                    emphasized: true,
                    ..button(ButtonId::ConfirmDelete, "Delete")
                },
            ],
            vertical: false,
        },
        Modal::FolderPicker {
            title,
            options,
            page,
            ..
        } => {
            let start = (page * PICKER_PAGE).min(options.len());
            let end = (start + PICKER_PAGE).min(options.len());
            let mut buttons: Vec<DialogButton> = options[start..end]
                .iter()
                .enumerate()
                .map(|(k, (_, name))| button(ButtonId::PickFolder(start + k), name))
                .collect();
            if options.len() > PICKER_PAGE {
                buttons.push(DialogButton {
                    enabled: start > 0,
                    ..button(ButtonId::PickerPrevious, "Previous")
                });
                buttons.push(DialogButton {
                    enabled: end < options.len(),
                    ..button(ButtonId::PickerNext, "Next")
                });
            }
            buttons.push(button(ButtonId::Cancel, "Cancel"));
            DialogSpec {
                heading: format!("Move \"{title}\" to"),
                body: Vec::new(),
                buttons,
                vertical: true,
            }
        }
        Modal::ConfirmDeleteNotebook { title, .. } => DialogSpec {
            heading: format!("Delete \"{title}\"?"),
            body: vec!["The notebook will be removed from the library.".into()],
            buttons: vec![
                button(ButtonId::Cancel, "Cancel"),
                DialogButton {
                    emphasized: true,
                    ..button(ButtonId::ConfirmDelete, "Delete")
                },
            ],
            vertical: false,
        },
        Modal::PageMenu {
            page_number,
            page_count,
        } => DialogSpec {
            heading: format!("Page {page_number} of {page_count}"),
            body: Vec::new(),
            buttons: vec![
                button(ButtonId::InsertPageAfter, "Insert page after"),
                DialogButton {
                    enabled: *page_number > 1,
                    ..button(ButtonId::MovePageLeft, "Move page left")
                },
                DialogButton {
                    enabled: page_number < page_count,
                    ..button(ButtonId::MovePageRight, "Move page right")
                },
                button(ButtonId::DeletePage, "Delete page"),
                button(ButtonId::Cancel, "Cancel"),
            ],
            vertical: true,
        },
        Modal::ConfirmDeletePage { page_number } => DialogSpec {
            heading: format!("Delete page {page_number}?"),
            body: Vec::new(),
            buttons: vec![
                button(ButtonId::Cancel, "Cancel"),
                DialogButton {
                    emphasized: true,
                    ..button(ButtonId::ConfirmDelete, "Delete")
                },
            ],
            vertical: false,
        },
        Modal::Notice { heading, body } => DialogSpec {
            heading: heading.clone(),
            body: body.clone(),
            buttons: vec![button(ButtonId::Ok, "OK")],
            vertical: false,
        },
        Modal::LinkJump {
            label, target_live, ..
        } => DialogSpec {
            heading: "Link".into(),
            body: Vec::new(),
            buttons: vec![
                DialogButton {
                    id: ButtonId::Jump,
                    label: label.clone(),
                    enabled: *target_live,
                    emphasized: true,
                },
                button(ButtonId::RemoveLink, "Remove link"),
            ],
            vertical: true,
        },
        Modal::Error { message, .. } => DialogSpec {
            heading: "Something went wrong".into(),
            body: vec![message.clone()],
            buttons: vec![button(ButtonId::Ok, "OK")],
            vertical: false,
        },
        Modal::TitleEntry { .. } => return None,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogLayout {
    pub frame: Rect,
    pub heading: Vec<(u32, String)>,
    pub body: Vec<(u32, String)>,
    pub buttons: Vec<(DialogButton, Rect)>,
}

impl DialogLayout {
    pub fn new(geo: &FbGeometry, spec: &DialogSpec) -> Self {
        let box_w = geo.xres.saturating_sub(2 * SHEET_MARGIN_X).min(SHEET_MAX_W);
        let inner_w = box_w.saturating_sub(2 * PAD);
        let heading_lines = sans_wrap(HEADING_PX, &spec.heading, true, inner_w, 3);
        let body_lines: Vec<String> = spec
            .body
            .iter()
            .flat_map(|p| sans_wrap(BODY_PX, p, false, inner_w, 4))
            .collect();
        let heading_h = heading_lines.len() as u32 * HEADING_LINE;
        let body_h = if body_lines.is_empty() {
            0
        } else {
            16 + body_lines.len() as u32 * BODY_LINE
        };
        let n = spec.buttons.len() as u32;
        let (buttons_h, gap_above) = if spec.vertical {
            (n * ROW_H, 24)
        } else {
            (PILL_H, PAD)
        };
        let box_h = PAD
            + heading_h
            + body_h
            + gap_above
            + buttons_h
            + PAD / 2
            + if spec.vertical { 0 } else { PAD / 2 };
        let box_x = (geo.xres.saturating_sub(box_w)) / 2;
        let box_y = geo.yres.saturating_sub(box_h) / 2;
        let frame = Rect::new(box_x, box_y, box_w, box_h);

        let mut y = box_y + PAD;
        let mut heading = Vec::new();
        for line in heading_lines {
            heading.push((y, line));
            y += HEADING_LINE;
        }
        let mut body = Vec::new();
        if !body_lines.is_empty() {
            y += 16;
            for line in body_lines {
                body.push((y, line));
                y += BODY_LINE;
            }
        }
        y += gap_above;
        let mut buttons = Vec::new();
        if spec.vertical {
            // Full-width rows (edge to edge inside the border) separated by hairlines.
            let x0 = box_x + BORDER;
            for b in &spec.buttons {
                buttons.push((b.clone(), Rect::new(x0, y, box_w - 2 * BORDER, ROW_H)));
                y += ROW_H;
            }
        } else if let Some(w) = inner_w
            .saturating_sub(n.saturating_sub(1) * BUTTON_GAP)
            .checked_div(n)
        {
            let x0 = box_x + PAD;
            for (i, b) in spec.buttons.iter().enumerate() {
                let x = x0 + i as u32 * (w + BUTTON_GAP);
                buttons.push((b.clone(), Rect::new(x, y, w, PILL_H)));
            }
        }
        Self {
            frame,
            heading,
            body,
            buttons,
        }
    }

    /// Enabled button under `p`. Disabled buttons and dialog chrome consume taps silently.
    pub fn hit(&self, p: UiPoint) -> Option<ButtonId> {
        self.buttons
            .iter()
            .find(|(b, r)| b.enabled && r.contains(p.x, p.y))
            .map(|(b, _)| b.id)
    }
}

/// Small line icon for a menu action; `None` draws nothing (labels that are user content).
fn draw_icon(c: &mut Canvas<'_>, id: ButtonId, r: Rect, ink: u8) {
    let (cx, cy) = ((r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32);
    let w = 5.0;
    match id {
        ButtonId::Rename => {
            // Pencil.
            c.line(cx - 18.0, cy + 18.0, cx + 14.0, cy - 14.0, 12.0, ink);
            c.line(cx - 18.0, cy + 18.0, cx - 24.0, cy + 24.0, 5.0, ink);
            c.line(cx + 8.0, cy - 20.0, cx + 20.0, cy - 8.0, 3.0, WHITE);
        }
        ButtonId::DeleteNotebook
        | ButtonId::DeletePage
        | ButtonId::DeleteFolder
        | ButtonId::ConfirmDelete => {
            // Trash can.
            c.line(cx - 22.0, cy - 16.0, cx + 22.0, cy - 16.0, w, ink);
            c.polyline(
                &[
                    (cx - 8.0, cy - 16.0),
                    (cx - 6.0, cy - 24.0),
                    (cx + 6.0, cy - 24.0),
                    (cx + 8.0, cy - 16.0),
                ],
                4.0,
                ink,
            );
            c.polyline(
                &[
                    (cx - 16.0, cy - 12.0),
                    (cx - 13.0, cy + 24.0),
                    (cx + 13.0, cy + 24.0),
                    (cx + 16.0, cy - 12.0),
                ],
                w,
                ink,
            );
            c.line(cx - 4.0, cy - 4.0, cx - 4.0, cy + 14.0, 3.0, ink);
            c.line(cx + 4.0, cy - 4.0, cx + 4.0, cy + 14.0, 3.0, ink);
        }
        ButtonId::AddToFolder
        | ButtonId::ChangeFolder
        | ButtonId::NewFolder
        | ButtonId::PickFolder(_) => {
            c.polyline(
                &[
                    (cx - 24.0, cy + 18.0),
                    (cx - 24.0, cy - 20.0),
                    (cx - 8.0, cy - 20.0),
                    (cx - 2.0, cy - 13.0),
                    (cx + 24.0, cy - 13.0),
                    (cx + 24.0, cy + 18.0),
                    (cx - 24.0, cy + 18.0),
                ],
                w,
                ink,
            );
            if matches!(id, ButtonId::NewFolder) {
                c.line(cx - 8.0, cy + 3.0, cx + 8.0, cy + 3.0, 4.0, ink);
                c.line(cx, cy - 5.0, cx, cy + 11.0, 4.0, ink);
            }
        }
        ButtonId::RemoveFromFolder => {
            c.polyline(
                &[
                    (cx - 24.0, cy + 18.0),
                    (cx - 24.0, cy - 20.0),
                    (cx - 8.0, cy - 20.0),
                    (cx - 2.0, cy - 13.0),
                    (cx + 24.0, cy - 13.0),
                    (cx + 24.0, cy + 18.0),
                    (cx - 24.0, cy + 18.0),
                ],
                w,
                ink,
            );
            c.line(cx - 8.0, cy + 3.0, cx + 8.0, cy + 3.0, 4.0, ink);
        }
        ButtonId::NewNotebook | ButtonId::InsertPageAfter => {
            c.polyline(
                &[
                    (cx - 18.0, cy - 26.0),
                    (cx + 18.0, cy - 26.0),
                    (cx + 18.0, cy + 26.0),
                    (cx - 18.0, cy + 26.0),
                    (cx - 18.0, cy - 26.0),
                ],
                w,
                ink,
            );
            c.line(cx - 9.0, cy, cx + 9.0, cy, 4.0, ink);
            c.line(cx, cy - 9.0, cx, cy + 9.0, 4.0, ink);
        }
        ButtonId::MovePageLeft | ButtonId::PickerPrevious => {
            c.line(cx - 20.0, cy, cx + 20.0, cy, w, ink);
            c.polyline(
                &[
                    (cx - 6.0, cy - 14.0),
                    (cx - 20.0, cy),
                    (cx - 6.0, cy + 14.0),
                ],
                w,
                ink,
            );
        }
        ButtonId::MovePageRight | ButtonId::PickerNext | ButtonId::Jump => {
            c.line(cx - 20.0, cy, cx + 20.0, cy, w, ink);
            c.polyline(
                &[
                    (cx + 6.0, cy - 14.0),
                    (cx + 20.0, cy),
                    (cx + 6.0, cy + 14.0),
                ],
                w,
                ink,
            );
        }
        ButtonId::RemoveLink => {
            c.line(cx - 20.0, cy + 8.0, cx - 6.0, cy - 6.0, 5.0, ink);
            c.line(cx + 6.0, cy + 6.0, cx + 20.0, cy - 8.0, 5.0, ink);
            c.line(cx - 18.0, cy - 18.0, cx + 18.0, cy + 18.0, 4.0, ink);
        }
        ButtonId::Cancel => {
            c.line(cx - 14.0, cy - 14.0, cx + 14.0, cy + 14.0, w, ink);
            c.line(cx - 14.0, cy + 14.0, cx + 14.0, cy - 14.0, w, ink);
        }
        ButtonId::Ok => {}
    }
}

/// Paint a dialog on top of whatever the framebuffer already shows: a rounded white sheet
/// with a flat offset shadow; menus are icon + label rows, confirmations are pill buttons.
pub fn render_dialog(
    fb: &mut [u8],
    geo: &FbGeometry,
    spec: &DialogSpec,
) -> Result<DialogLayout, String> {
    let layout = DialogLayout::new(geo, spec);
    let mut c = Canvas::new(fb, geo)?;
    let f = layout.frame;
    // A white halo separates the sheet from busy page content underneath.
    let halo = Rect::new(
        f.x.saturating_sub(28),
        f.y.saturating_sub(28),
        f.w + 56 + SHADOW,
        f.h + 56 + SHADOW,
    );
    c.fill_round(halo, RADIUS + 28.0, WHITE);
    c.fill_round(
        Rect::new(f.x + SHADOW, f.y + SHADOW, f.w, f.h),
        RADIUS,
        BLACK,
    );
    c.frame_round(f, RADIUS, BORDER, BLACK, WHITE);
    let text_x = f.x + PAD;
    let text_w = f.w.saturating_sub(2 * PAD);
    for (y, line) in &layout.heading {
        c.sans_left(
            Rect::new(text_x, *y, text_w, HEADING_LINE),
            HEADING_PX,
            line,
            BLACK,
            true,
        );
    }
    for (y, line) in &layout.body {
        c.sans_left(
            Rect::new(text_x, *y, text_w, BODY_LINE),
            BODY_PX,
            line,
            DARK,
            false,
        );
    }
    let last = layout.buttons.len().saturating_sub(1);
    for (i, (b, r)) in layout.buttons.iter().enumerate() {
        let ink = if b.enabled { BLACK } else { GRAY };
        if spec.vertical {
            // Hairline above each row; Cancel sits apart below a heavier rule.
            let cancel = b.id == ButtonId::Cancel && i == last && i > 0;
            c.fill(
                Rect::new(r.x, r.y, r.w, if cancel { 4 } else { 2 }),
                if cancel { BLACK } else { GRAY },
            );
            let mut fg = ink;
            if b.emphasized && b.enabled {
                let pill = Rect::new(r.x + PAD - 20, r.y + 14, r.w - 2 * (PAD - 20), r.h - 28);
                c.fill_round(pill, (pill.h / 2) as f32, BLACK);
                fg = WHITE;
            }
            let icon = Rect::new(
                r.x + PAD - 10,
                r.y + (r.h - ICON_BOX) / 2,
                ICON_BOX,
                ICON_BOX,
            );
            draw_icon(&mut c, b.id, icon, fg);
            let label_x = icon.right() + 24;
            c.sans_left(
                Rect::new(label_x, r.y, r.right().saturating_sub(label_x + PAD), r.h),
                LABEL_PX,
                &b.label,
                fg,
                !cancel,
            );
        } else {
            let radius = (r.h / 2) as f32;
            let fg = if !b.enabled {
                c.frame_round(*r, radius, 4, GRAY, WHITE);
                GRAY
            } else if b.emphasized {
                c.fill_round(*r, radius, BLACK);
                WHITE
            } else {
                c.frame_round(*r, radius, 4, BLACK, WHITE);
                BLACK
            };
            c.sans_centered(r.inset(16), LABEL_PX, &b.label, fg, true);
        }
    }
    Ok(layout)
}
