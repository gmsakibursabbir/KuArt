//! Editor UI drawn with egui around the canvas, Figma-style: a top bar with the KuArt menu,
//! tools and actions; pages and layers on the left; Design and Dev tabs on the right; a status
//! bar; and a home screen with recent files. Panels read the document and return [`Action`]s;
//! the app applies them through history.

mod comments;
mod devmode;
mod home;
pub mod icons;
mod inspector;
mod layers;
pub mod theme;
pub mod widgets;

use std::path::PathBuf;

pub use egui;
pub use home::Recent;
pub use layers::{LayersPanel, ROW_HEIGHT};
pub use theme::{ThemeChoice, Tokens};

use egui::{Align, Align2, FontId, Layout, Panel, Rect, RichText, Ui};
use inspector::Inspector;
use ude_core::{Document, NodeId, Prop};
use ude_editor::{Selection, Tool};
use widgets::{icon, icon_button, icon_button_sized, primary_button, segmented};

/// Boolean operations: icon, label, shortcut, command.
pub const BOOLEANS: [(&str, &str, &str, Edit); 4] = [
    (icons::SQUARES_UNITE, "Union", "Ctrl+Alt+U", Edit::Union),
    (
        icons::SQUARES_SUBTRACT,
        "Subtract",
        "Ctrl+Alt+S",
        Edit::Subtract,
    ),
    (
        icons::SQUARES_INTERSECT,
        "Intersect",
        "Ctrl+Alt+I",
        Edit::Intersect,
    ),
    (
        icons::SQUARES_EXCLUDE,
        "Exclude",
        "Ctrl+Alt+X",
        Edit::Exclude,
    ),
];

/// Structural edit commands (menu and shortcuts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Copy,
    Cut,
    Paste,
    Duplicate,
    Delete,
    Group,
    Ungroup,
    /// Boolean groups (live) and flattening them into a vector.
    Union,
    Subtract,
    Intersect,
    Exclude,
    Flatten,
    /// Group with the bottom layer as a mask.
    Mask,
    BringForward,
    SendBackward,
    SelectAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Png,
    Jpeg,
    WebP,
    Svg,
    Pdf,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 5] = [Self::Png, Self::Jpeg, Self::WebP, Self::Svg, Self::Pdf];

    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPG",
            Self::WebP => "WebP",
            Self::Svg => "SVG",
            Self::Pdf => "PDF",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::WebP => "webp",
            Self::Svg => "svg",
            Self::Pdf => "pdf",
        }
    }

    pub fn is_raster(self) -> bool {
        matches!(self, Self::Png | Self::Jpeg | Self::WebP)
    }
}

/// Export settings from the Design tab.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExportOptions {
    pub format: ExportFormat,
    pub scale: f32,
    /// Smaller files: palette PNG/WebP; JPEG uses `quality`.
    pub compress: bool,
    /// 10–100.
    pub quality: u8,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Png,
            scale: 2.0,
            compress: false,
            quality: 85,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Select(Vec<NodeId>),
    Toggle(NodeId),
    Hover(Option<NodeId>),
    /// `merge_key` 0 means "its own undo step".
    Set {
        id: NodeId,
        value: Prop,
        merge_key: u64,
    },
    /// Text edits; the app interns the family and re-measures auto-sized boxes.
    SetText {
        id: NodeId,
        content: Option<String>,
        family: Option<String>,
        size: Option<f32>,
        weight: Option<u16>,
        merge_key: u64,
    },
    Rename(NodeId, String),
    Tool(Tool),
    Edit(Edit),
    New,
    Open,
    Import,
    OpenRecent(PathBuf),
    RemoveRecent(PathBuf),
    /// Close the document and show the home screen.
    Home,
    Save,
    SaveAs,
    /// Export dialog with the last options.
    Export,
    ExportWith(ExportOptions),
    SelectPage(NodeId),
    AddPage,
    /// Download a missing font family from Google Fonts (user-initiated).
    DownloadFont(String),
    /// Help → Check for updates (GitHub Releases), and installing the update found.
    CheckUpdates,
    InstallUpdate,
    Undo,
    Redo,
    ZoomToFit,
    ZoomIn,
    ZoomOut,
    Zoom100,
    ToggleDiagnostics,
    SwitchTab(usize),
    CloseTab(usize),
    Align(ude_editor::Align),
    /// New comment pinned at the centre of the view.
    AddComment(String),
    ReplyComment(usize, String),
    ResolveComment(usize, bool),
    DeleteComment(usize),
    /// Centre the view on a comment.
    FocusComment(usize),
}

/// One open document in the tab strip.
pub struct TabInfo {
    pub title: String,
    pub dirty: bool,
}

/// Read-only facts the bars display.
pub struct Status<'a> {
    pub title: &'a str,
    pub dirty: bool,
    pub zoom: f64,
    pub renderer: &'a str,
    pub can_undo: bool,
    pub can_redo: bool,
    pub tool: Tool,
    /// Font families used by the document but not installed.
    pub missing_fonts: &'a [&'a str],
    /// Families of the missing faces (each gets a Google Fonts download button).
    pub missing_families: &'a [&'a str],
    /// Progress or result of the last font download.
    pub font_status: Option<&'a str>,
    /// Rows for the diagnostics window (F12); `None` hides it.
    pub diagnostics: Option<&'a [(&'a str, String)]>,
    pub recents: &'a [Recent],
    /// Show the home screen instead of the editor.
    pub home: bool,
    pub tabs: &'a [TabInfo],
    /// Selected tab; `None` while the home screen is shown.
    pub active_tab: Option<usize>,
    /// What the current tool or mode does (status bar).
    pub hint: Option<&'a str>,
    /// A newer release: version, release page, and whether it can install itself.
    pub update: Option<(&'a str, &'a str, bool)>,
    /// Progress or result of the last update check or install.
    pub update_status: Option<&'a str>,
}

#[derive(Default)]
pub struct Panels {
    pub theme: ThemeChoice,
    pub layers: LayersPanel,
    /// Right panel shows Dev Mode (code and inspect) instead of Design.
    pub dev_mode: bool,
    /// Right panel shows comments.
    pub comments_open: bool,
    inspector: Inspector,
    comments: comments::CommentsPanel,
    dev: devmode::DevMode,
    home: home::Home,
}

const TOOL_ICONS: [(Tool, &str); 9] = [
    (Tool::Move, icons::MOUSE_POINTER_2),
    (Tool::Frame, icons::FRAME),
    (Tool::Rectangle, icons::SQUARE),
    (Tool::Ellipse, icons::CIRCLE),
    (Tool::Polygon, icons::TRIANGLE),
    (Tool::Star, icons::STAR),
    (Tool::Line, icons::SLASH),
    (Tool::Pen, icons::PEN_TOOL),
    (Tool::Text, icons::TYPE),
];

impl Panels {
    /// Export options last used in the Design tab.
    pub fn export_options(&self) -> ExportOptions {
        self.inspector.export
    }

    /// Draws the UI into `root` and returns the actions plus the canvas rect (points).
    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        root: &mut Ui,
        doc: &Document,
        page: NodeId,
        selection: &Selection,
        hover: Option<NodeId>,
        status: &Status<'_>,
        t: &Tokens,
    ) -> (Vec<Action>, Rect) {
        let mut actions = Vec::new();
        tab_strip(root, status, t, &mut actions);
        if status.home {
            self.home
                .ui(root, status.recents, &mut self.theme, t, &mut actions);
            return (actions, Rect::NOTHING);
        }

        self.top_bar(root, doc, page, status, t, &mut actions);
        self.status_bar(root, selection, status, t, &mut actions);
        diagnostics(root, status, t);

        Panel::left("left")
            .default_size(248.0)
            .size_range(180.0..=480.0)
            .frame(
                egui::Frame::new()
                    .fill(t.bg_panel)
                    .inner_margin(egui::Margin::symmetric(8, 6)),
            )
            .show_inside(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Pages")
                            .size(11.5)
                            .strong()
                            .color(t.text_primary),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button_sized(ui, t, icons::PLUS, "Add page", false, 22.0, 13.0)
                            .clicked()
                        {
                            actions.push(Action::AddPage);
                        }
                    });
                });
                egui::ScrollArea::vertical()
                    .id_salt("pages")
                    .max_height(132.0)
                    .show(ui, |ui| {
                        self.layers.pages_ui(ui, doc, page, t, &mut actions);
                    });
                ui.add_space(6.0);
                let (line, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 1.0),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(line, 0.0, t.border_subtle);
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Layers")
                        .size(11.5)
                        .strong()
                        .color(t.text_primary),
                );
                ui.add_space(2.0);
                self.layers
                    .ui(ui, doc, page, selection, hover, t, &mut actions);
            });

        Panel::right("right")
            .default_size(288.0)
            .size_range(240.0..=520.0)
            .frame(
                egui::Frame::new()
                    .fill(t.bg_panel)
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show_inside(root, |ui| {
                let mut tab = if self.comments_open {
                    2
                } else {
                    usize::from(self.dev_mode)
                };
                let open = doc.comments.iter().filter(|c| !c.resolved).count();
                let comments_label = if open > 0 {
                    format!("Comments {open}")
                } else {
                    "Comments".to_owned()
                };
                segmented(ui, t, &["Design", "Dev Mode", &comments_label], &mut tab);
                self.dev_mode = tab == 1;
                self.comments_open = tab == 2;
                egui::ScrollArea::vertical()
                    .id_salt("right_scroll")
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        if self.comments_open {
                            let page_index = doc
                                .children(doc.root())
                                .iter()
                                .position(|&p| p == page)
                                .unwrap_or(0) as u32;
                            self.comments.ui(ui, doc, page_index, t, &mut actions);
                        } else if self.dev_mode {
                            self.dev.ui(ui, doc, selection, page, t);
                        } else {
                            self.inspector.ui(ui, doc, selection, t, &mut actions);
                        }
                    });
            });

        let canvas = root.available_rect_before_wrap();
        tool_pill(root.ctx(), canvas, status.tool, t, &mut actions);
        (actions, canvas)
    }

    fn top_bar(
        &mut self,
        root: &mut Ui,
        doc: &Document,
        page: NodeId,
        status: &Status<'_>,
        t: &Tokens,
        actions: &mut Vec<Action>,
    ) {
        Panel::top("top_bar")
            .exact_size(46.0)
            .frame(
                egui::Frame::new()
                    .fill(t.bg_app)
                    .inner_margin(egui::Margin::symmetric(8, 0)),
            )
            .show_inside(root, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    self.main_menu(ui, status, t, actions);

                    // Centre: document and page.
                    let page_name = doc.node(page).map_or("", |n| doc.strings.resolve(n.name));
                    let title = format!("{}{}", status.title, if status.dirty { " •" } else { "" });
                    let center = ui.max_rect().center();
                    let p = ui.painter();
                    p.text(
                        egui::pos2(center.x, center.y),
                        Align2::CENTER_CENTER,
                        if page_name.is_empty() {
                            title
                        } else {
                            format!("{title}   /   {page_name}")
                        },
                        FontId::proportional(12.5),
                        t.text_secondary,
                    );

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if primary_button(ui, t, Some(icons::DOWNLOAD), "Export")
                            .on_hover_text("Export selection or page (Ctrl+Shift+E)")
                            .clicked()
                        {
                            actions.push(Action::Export);
                        }
                        if icon_button(
                            ui,
                            t,
                            icons::CODE_XML,
                            "Dev Mode: inspect and copy code",
                            self.dev_mode,
                        )
                        .clicked()
                        {
                            self.dev_mode = !self.dev_mode;
                        }
                        let (glyph, next, tip) = if t.dark {
                            (icons::SUN, ThemeChoice::Light, "Light theme")
                        } else {
                            (icons::MOON, ThemeChoice::Dark, "Dark theme")
                        };
                        if icon_button(ui, t, glyph, tip, false).clicked() {
                            self.theme = next;
                        }
                        ui.add_space(4.0);
                        ui.menu_button(
                            RichText::new(format!("{:.0}%", status.zoom * 100.0))
                                .color(t.text_secondary),
                            |ui| {
                                menu_item(
                                    ui,
                                    icons::ZOOM_IN,
                                    "Zoom in",
                                    "Ctrl+=",
                                    actions,
                                    Action::ZoomIn,
                                );
                                menu_item(
                                    ui,
                                    icons::ZOOM_OUT,
                                    "Zoom out",
                                    "Ctrl+-",
                                    actions,
                                    Action::ZoomOut,
                                );
                                menu_item(
                                    ui,
                                    icons::MAXIMIZE,
                                    "Zoom to fit",
                                    "Shift+1",
                                    actions,
                                    Action::ZoomToFit,
                                );
                                menu_item(
                                    ui,
                                    icons::SCAN_SEARCH,
                                    "Zoom to 100%",
                                    "Ctrl+0",
                                    actions,
                                    Action::Zoom100,
                                );
                            },
                        );
                        ui.add_enabled_ui(status.can_redo, |ui| {
                            if icon_button(ui, t, icons::REDO_2, "Redo  Ctrl+Y", false).clicked() {
                                actions.push(Action::Redo);
                            }
                        });
                        ui.add_enabled_ui(status.can_undo, |ui| {
                            if icon_button(ui, t, icons::UNDO_2, "Undo  Ctrl+Z", false).clicked() {
                                actions.push(Action::Undo);
                            }
                        });
                    });
                });
            });
    }

    /// Logo and the global menu bar: File, Edit, View, Object, Help.
    fn main_menu(
        &mut self,
        ui: &mut Ui,
        status: &Status<'_>,
        t: &Tokens,
        actions: &mut Vec<Action>,
    ) {
        let (r, logo) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
        ui.painter().rect_filled(r, 7.0, t.accent);
        ui.painter().text(
            r.center(),
            Align2::CENTER_CENTER,
            "K",
            FontId::proportional(14.0),
            t.on_accent,
        );
        if logo.on_hover_text("Back to files").clicked() {
            actions.push(Action::Home);
        }
        ui.add_space(6.0);
        // Menu titles are plain text until hovered or open, like desktop menu bars.
        let w = &mut ui.visuals_mut().widgets;
        w.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
        w.inactive.bg_fill = egui::Color32::TRANSPARENT;
        let menu = |label: &str| RichText::new(label).size(12.5).color(t.text_secondary);

        ui.menu_button(menu("File"), |ui| {
            ui.set_width(220.0);
            menu_item(
                ui,
                icons::FILE_PLUS,
                "New design",
                "Ctrl+N",
                actions,
                Action::New,
            );
            menu_item(
                ui,
                icons::FOLDER_OPEN,
                "Open…",
                "Ctrl+O",
                actions,
                Action::Open,
            );
            menu_item(
                ui,
                icons::FILE_INPUT,
                "Import…",
                "",
                actions,
                Action::Import,
            );
            ui.menu_button(submenu(ui, icons::CLOCK, "Open recent"), |ui| {
                if status.recents.is_empty() {
                    ui.label(RichText::new("No recent files").color(t.text_muted));
                }
                for r in status.recents.iter().take(12) {
                    if ui
                        .button(&r.name)
                        .on_hover_text(r.path.display().to_string())
                        .clicked()
                    {
                        actions.push(Action::OpenRecent(r.path.clone()));
                        ui.close();
                    }
                }
            });
            ui.separator();
            menu_item(ui, icons::SAVE, "Save", "Ctrl+S", actions, Action::Save);
            menu_item(
                ui,
                icons::SAVE_ALL,
                "Save as…",
                "Ctrl+Shift+S",
                actions,
                Action::SaveAs,
            );
            menu_item(
                ui,
                icons::DOWNLOAD,
                "Export…",
                "Ctrl+Shift+E",
                actions,
                Action::Export,
            );
            ui.separator();
            menu_item(
                ui,
                icons::FILE_PLUS_2,
                "Add page",
                "",
                actions,
                Action::AddPage,
            );
            ui.separator();
            menu_item(ui, icons::HOUSE, "Back to files", "", actions, Action::Home);
        });
        ui.menu_button(menu("Edit"), |ui| {
            ui.set_width(220.0);
            menu_item(ui, icons::UNDO_2, "Undo", "Ctrl+Z", actions, Action::Undo);
            menu_item(ui, icons::REDO_2, "Redo", "Ctrl+Y", actions, Action::Redo);
            ui.separator();
            menu_item(
                ui,
                icons::COPY,
                "Copy",
                "Ctrl+C",
                actions,
                Action::Edit(Edit::Copy),
            );
            menu_item(
                ui,
                icons::SCISSORS,
                "Cut",
                "Ctrl+X",
                actions,
                Action::Edit(Edit::Cut),
            );
            menu_item(
                ui,
                icons::CLIPBOARD_PASTE,
                "Paste",
                "Ctrl+V",
                actions,
                Action::Edit(Edit::Paste),
            );
            menu_item(
                ui,
                icons::COPY_PLUS,
                "Duplicate",
                "Ctrl+D",
                actions,
                Action::Edit(Edit::Duplicate),
            );
            menu_item(
                ui,
                icons::TRASH_2,
                "Delete",
                "Del",
                actions,
                Action::Edit(Edit::Delete),
            );
            ui.separator();
            menu_item(
                ui,
                icons::SQUARE_DASHED_MOUSE_POINTER,
                "Select all",
                "Ctrl+A",
                actions,
                Action::Edit(Edit::SelectAll),
            );
        });
        ui.menu_button(menu("View"), |ui| {
            ui.set_width(220.0);
            menu_item(
                ui,
                icons::ZOOM_IN,
                "Zoom in",
                "Ctrl+=",
                actions,
                Action::ZoomIn,
            );
            menu_item(
                ui,
                icons::ZOOM_OUT,
                "Zoom out",
                "Ctrl+-",
                actions,
                Action::ZoomOut,
            );
            menu_item(
                ui,
                icons::MAXIMIZE,
                "Zoom to fit",
                "Shift+1",
                actions,
                Action::ZoomToFit,
            );
            menu_item(
                ui,
                icons::SCAN_SEARCH,
                "Zoom to 100%",
                "Ctrl+0",
                actions,
                Action::Zoom100,
            );
            ui.separator();
            if ui.checkbox(&mut self.dev_mode, "Dev Mode").clicked() {
                ui.close();
            }
            menu_item(
                ui,
                icons::ACTIVITY,
                "Diagnostics",
                "F12",
                actions,
                Action::ToggleDiagnostics,
            );
            ui.separator();
            ui.label(RichText::new("Theme").size(11.0).color(t.text_muted));
            for c in ThemeChoice::ALL {
                if ui.radio(self.theme == c, c.label()).clicked() {
                    self.theme = c;
                    ui.close();
                }
            }
        });
        ui.menu_button(menu("Object"), |ui| {
            ui.set_width(220.0);
            menu_item(
                ui,
                icons::GROUP,
                "Group",
                "Ctrl+G",
                actions,
                Action::Edit(Edit::Group),
            );
            menu_item(
                ui,
                icons::UNGROUP,
                "Ungroup",
                "Ctrl+Shift+G",
                actions,
                Action::Edit(Edit::Ungroup),
            );
            ui.separator();
            for (glyph, label, key, e) in BOOLEANS {
                menu_item(ui, glyph, label, key, actions, Action::Edit(e));
            }
            menu_item(
                ui,
                icons::COMBINE,
                "Flatten",
                "Ctrl+E",
                actions,
                Action::Edit(Edit::Flatten),
            );
            menu_item(
                ui,
                icons::SCAN_EYE,
                "Use as mask",
                "Ctrl+Alt+M",
                actions,
                Action::Edit(Edit::Mask),
            );
            ui.separator();
            menu_item(
                ui,
                icons::BRING_TO_FRONT,
                "Bring forward",
                "Ctrl+]",
                actions,
                Action::Edit(Edit::BringForward),
            );
            menu_item(
                ui,
                icons::SEND_TO_BACK,
                "Send backward",
                "Ctrl+[",
                actions,
                Action::Edit(Edit::SendBackward),
            );
            ui.separator();
            ui.menu_button(submenu(ui, icons::ALIGN_CENTER_VERTICAL, "Align"), |ui| {
                use ude_editor::Align as A;
                menu_item(
                    ui,
                    icons::ALIGN_START_VERTICAL,
                    "Left",
                    "Alt+A",
                    actions,
                    Action::Align(A::Left),
                );
                menu_item(
                    ui,
                    icons::ALIGN_CENTER_VERTICAL,
                    "Horizontal centers",
                    "Alt+H",
                    actions,
                    Action::Align(A::HCenter),
                );
                menu_item(
                    ui,
                    icons::ALIGN_END_VERTICAL,
                    "Right",
                    "Alt+D",
                    actions,
                    Action::Align(A::Right),
                );
                ui.separator();
                menu_item(
                    ui,
                    icons::ALIGN_START_HORIZONTAL,
                    "Top",
                    "Alt+W",
                    actions,
                    Action::Align(A::Top),
                );
                menu_item(
                    ui,
                    icons::ALIGN_CENTER_HORIZONTAL,
                    "Vertical centers",
                    "Alt+V",
                    actions,
                    Action::Align(A::VCenter),
                );
                menu_item(
                    ui,
                    icons::ALIGN_END_HORIZONTAL,
                    "Bottom",
                    "Alt+S",
                    actions,
                    Action::Align(A::Bottom),
                );
                ui.separator();
                menu_item(
                    ui,
                    icons::ALIGN_HORIZONTAL_SPACE_BETWEEN,
                    "Distribute horizontally",
                    "Alt+Shift+H",
                    actions,
                    Action::Align(A::DistributeH),
                );
                menu_item(
                    ui,
                    icons::ALIGN_VERTICAL_SPACE_BETWEEN,
                    "Distribute vertically",
                    "Alt+Shift+V",
                    actions,
                    Action::Align(A::DistributeV),
                );
            });
        });
        ui.menu_button(menu("Help"), |ui| {
            ui.set_width(250.0);
            ui.label(RichText::new(concat!("KuArt ", env!("CARGO_PKG_VERSION"))).strong());
            ui.label(
                RichText::new("Native, offline design editor")
                    .size(11.0)
                    .color(t.text_muted),
            );
            ui.separator();
            ui.label(RichText::new("Tools: V F R O P S L T").size(11.5));
            ui.label(RichText::new("Pan: Space+drag or wheel · Zoom: Ctrl+wheel").size(11.5));
            ui.label(RichText::new("Diagnostics: F12 · Stats: F3").size(11.5));
            ui.separator();
            menu_item(
                ui,
                icons::ROTATE_CW,
                "Check for updates…",
                "",
                actions,
                Action::CheckUpdates,
            );
            ui.separator();
            ui.hyperlink_to(
                "Documentation and source",
                "https://github.com/gmsakibursabbir/KuArt",
            );
            ui.hyperlink_to(
                "Report a problem",
                "https://github.com/gmsakibursabbir/KuArt/issues",
            );
        });
    }

    fn status_bar(
        &mut self,
        root: &mut Ui,
        selection: &Selection,
        status: &Status<'_>,
        t: &Tokens,
        actions: &mut Vec<Action>,
    ) {
        Panel::bottom("status_bar")
            .exact_size(26.0)
            .frame(egui::Frame::new().fill(t.bg_app).inner_margin(egui::Margin::symmetric(10, 0)))
            .show_inside(root, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new(format!("{} layers", self.layers.rows().len())).size(11.0).color(t.text_muted));
                    if !selection.is_empty() {
                        ui.label(RichText::new(format!("· {} selected", selection.ids().len())).size(11.0).color(t.text_muted));
                    }
                    if let Some(h) = status.hint {
                        ui.add_space(8.0);
                        ui.label(icon(icons::INFO, 12.0, t.accent));
                        ui.label(RichText::new(h).size(11.0).color(t.text_secondary));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(status.renderer).size(11.0).color(t.text_muted));
                        if let Some((version, url, installable)) = status.update {
                            if installable
                                && ui
                                    .add(egui::Button::new(RichText::new(format!("Install {version}")).size(11.0).color(t.on_accent)).fill(t.accent))
                                    .on_hover_text("Download, verify and install; you are asked to save first")
                                    .clicked()
                            {
                                actions.push(Action::InstallUpdate);
                            }
                            ui.hyperlink_to(RichText::new("What's new").size(11.0), url);
                        }
                        if let Some(s) = status.update_status {
                            ui.label(RichText::new(s).size(11.0).color(t.text_muted));
                        }
                        if !status.missing_fonts.is_empty() {
                            for family in status.missing_families {
                                if ui
                                    .small_button(format!("Get {family}"))
                                    .on_hover_text("Download from Google Fonts into the KuArt fonts folder")
                                    .clicked()
                                {
                                    actions.push(Action::DownloadFont((*family).to_owned()));
                                }
                            }
                            let list = status.missing_fonts.join(", ");
                            ui.label(RichText::new(format!("Missing fonts: {list}")).size(11.0).color(t.warning))
                                .on_hover_text("Drawn with a fallback font. The original font name is kept when saving.");
                            ui.label(icon(icons::TRIANGLE_ALERT, 12.0, t.warning));
                        }
                        if let Some(s) = status.font_status {
                            ui.label(RichText::new(s).size(11.0).color(t.text_muted));
                        }
                    });
                });
            });
    }
}

/// Submenu title with an icon, aligned with [`menu_item`] rows.
fn submenu(ui: &Ui, glyph: &str, label: &str) -> egui::text::LayoutJob {
    let v = ui.visuals();
    let mut job = egui::text::LayoutJob::default();
    job.append(
        glyph,
        0.0,
        egui::TextFormat::simple(widgets::icon_font(14.0), v.weak_text_color()),
    );
    job.append(
        label,
        9.0,
        egui::TextFormat::simple(FontId::proportional(12.5), v.text_color()),
    );
    job
}

/// Menu entry: icon, label, and the shortcut right-aligned, across the menu width.
fn menu_item(
    ui: &mut Ui,
    glyph: &str,
    label: &str,
    shortcut: &str,
    actions: &mut Vec<Action>,
    action: Action,
) {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
    let v = ui.visuals();
    let (text, weak) = (v.text_color(), v.weak_text_color());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 4.0, v.widgets.hovered.weak_bg_fill);
    }
    let p = ui.painter();
    p.text(
        egui::pos2(rect.left() + 14.0, rect.center().y),
        Align2::CENTER_CENTER,
        glyph,
        widgets::icon_font(14.0),
        weak,
    );
    p.text(
        egui::pos2(rect.left() + 30.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.5),
        text,
    );
    if !shortcut.is_empty() {
        p.text(
            egui::pos2(rect.right() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            shortcut,
            FontId::proportional(11.0),
            weak,
        );
    }
    if resp.clicked() {
        actions.push(action);
        ui.close();
    }
}

fn diagnostics(root: &mut Ui, status: &Status<'_>, t: &Tokens) {
    let Some(rows) = status.diagnostics else {
        return;
    };
    egui::Window::new("Diagnostics")
        .default_pos(egui::pos2(280.0, 70.0))
        .resizable(false)
        .collapsible(true)
        .show(root.ctx(), |ui| {
            egui::Grid::new("diagnostics")
                .num_columns(2)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for (label, value) in rows {
                        ui.colored_label(t.text_muted, *label);
                        ui.monospace(value);
                        ui.end_row();
                    }
                });
            ui.colored_label(t.text_muted, "F12 to close");
        });
}

/// Document tabs above everything (Figma desktop style): Home, one tab per file, +.
fn tab_strip(root: &mut Ui, status: &Status<'_>, t: &Tokens, actions: &mut Vec<Action>) {
    Panel::top("tabs")
        .exact_size(34.0)
        .frame(
            egui::Frame::new()
                .fill(t.bg_app)
                .inner_margin(egui::Margin {
                    left: 6,
                    right: 6,
                    top: 4,
                    bottom: 0,
                }),
        )
        .show_inside(root, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let home_active = status.active_tab.is_none();
                if tab(ui, t, Some(icons::HOUSE), "", false, home_active, None)
                    .0
                    .clicked()
                {
                    actions.push(Action::Home);
                }
                for (i, info) in status.tabs.iter().enumerate() {
                    let (resp, close) = tab(
                        ui,
                        t,
                        None,
                        &info.title,
                        info.dirty,
                        status.active_tab == Some(i),
                        Some(i),
                    );
                    if close {
                        actions.push(Action::CloseTab(i));
                    } else if resp.clicked() {
                        actions.push(Action::SwitchTab(i));
                    }
                    if resp.middle_clicked() {
                        actions.push(Action::CloseTab(i));
                    }
                }
                if icon_button_sized(ui, t, icons::PLUS, "New design  Ctrl+N", false, 26.0, 14.0)
                    .clicked()
                {
                    actions.push(Action::New);
                }
            });
        });
}

/// One tab; returns its response and whether its close button was clicked.
fn tab(
    ui: &mut Ui,
    t: &Tokens,
    glyph: Option<&str>,
    title: &str,
    dirty: bool,
    active: bool,
    closable: Option<usize>,
) -> (egui::Response, bool) {
    let font = FontId::proportional(12.0);
    let text_w = if title.is_empty() {
        0.0
    } else {
        ui.fonts_mut(|f| {
            f.layout_no_wrap(title.to_owned(), font.clone(), t.text_primary)
                .size()
                .x
        })
        .min(180.0)
    };
    let w = if glyph.is_some() { 36.0 } else { text_w + 52.0 };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 30.0), egui::Sense::click());
    let p = ui.painter();
    let bg = if active {
        t.bg_panel
    } else if resp.hovered() {
        t.bg_hover
    } else {
        egui::Color32::TRANSPARENT
    };
    p.rect_filled(
        rect,
        egui::CornerRadius {
            nw: 7,
            ne: 7,
            sw: 0,
            se: 0,
        },
        bg,
    );
    let color = if active {
        t.text_primary
    } else {
        t.text_secondary
    };
    if let Some(g) = glyph {
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            g,
            widgets::icon_font(15.0),
            color,
        );
        return (resp.on_hover_text("Home: recent files"), false);
    }
    let text_rect = Rect::from_min_max(
        egui::pos2(rect.left() + 12.0, rect.top()),
        egui::pos2(rect.right() - 34.0, rect.bottom()),
    );
    p.with_clip_rect(text_rect).text(
        egui::pos2(text_rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        title,
        font,
        color,
    );
    // Close button; an unsaved dot until hovered.
    let close_rect = Rect::from_center_size(
        egui::pos2(rect.right() - 16.0, rect.center().y),
        egui::vec2(20.0, 20.0),
    );
    let mut closed = false;
    if closable.is_some() {
        let close = ui.interact(close_rect, resp.id.with("close"), egui::Sense::click());
        if dirty && !resp.hovered() && !close.hovered() {
            ui.painter()
                .circle_filled(close_rect.center(), 3.5, t.text_secondary);
        } else if resp.hovered() || close.hovered() || active {
            if close.hovered() {
                ui.painter().rect_filled(close_rect, 5.0, t.bg_hover);
            }
            ui.painter().text(
                close_rect.center(),
                Align2::CENTER_CENTER,
                icons::X,
                widgets::icon_font(12.0),
                t.text_secondary,
            );
        }
        closed = close.on_hover_text("Close  Ctrl+W").clicked();
    }
    (resp.on_hover_text(title.to_owned()), closed)
}

/// Floating tool bar centred at the bottom of the canvas (Figma UI3 style).
fn tool_pill(
    ctx: &egui::Context,
    canvas: Rect,
    current: Tool,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    if canvas.width() < 120.0 || canvas.height() < 80.0 {
        return;
    }
    egui::Area::new(egui::Id::new("tool_pill"))
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(egui::pos2(canvas.center().x, canvas.bottom() - 16.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(t.bg_panel)
                .stroke(egui::Stroke::new(1.0_f32, t.border_subtle))
                .corner_radius(14.0)
                .inner_margin(egui::Margin::symmetric(6, 5))
                .shadow(egui::Shadow {
                    offset: [0, 4],
                    blur: 16,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(if t.dark { 120 } else { 40 }),
                })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        for (i, (tool, glyph)) in TOOL_ICONS.into_iter().enumerate() {
                            // Separators between select, containers, shapes and text.
                            if matches!(i, 1 | 2 | 7) {
                                let (r, _) = ui.allocate_exact_size(
                                    egui::vec2(9.0, 24.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().vline(
                                    r.center().x,
                                    r.y_range().shrink(3.0),
                                    egui::Stroke::new(1.0_f32, t.border_subtle),
                                );
                            }
                            let tip = match tool.shortcut() {
                                Some(k) => format!("{}  {k}", tool.label()),
                                None => tool.label().to_owned(),
                            };
                            if widgets::icon_button_sized(
                                ui,
                                t,
                                glyph,
                                &tip,
                                current == tool,
                                34.0,
                                17.0,
                            )
                            .clicked()
                            {
                                actions.push(Action::Tool(tool));
                            }
                        }
                    });
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ude_core::{Node, NodeKind, Symbol};

    fn doc_with(n: usize) -> (Document, NodeId) {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        for _ in 0..n {
            doc.build_child(page, Node::new(NodeKind::Rectangle, Symbol::EMPTY))
                .unwrap();
        }
        (doc, page)
    }

    /// Headless frame: panels lay out only visible rows whatever the layer count.
    fn frame(panels: &mut Panels, doc: &Document, page: NodeId, home: bool) -> (Vec<Action>, Rect) {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let mut out = None;
        let status = Status {
            title: "t",
            dirty: false,
            zoom: 1.0,
            renderer: "cpu",
            can_undo: false,
            can_redo: false,
            tool: ude_editor::Tool::Move,
            missing_fonts: &[],
            missing_families: &[],
            font_status: None,
            diagnostics: None,
            recents: &[],
            home,
            tabs: &[],
            active_tab: None,
            hint: Some("Drag to draw"),
            update: Some((
                "9.9.9",
                "https://github.com/gmsakibursabbir/KuArt/releases/latest",
                true,
            )),
            update_status: Some("KuArt 9.9.9 is available"),
        };
        let _ = ctx.run_ui(input, |ui| {
            out = Some(panels.ui(
                ui,
                doc,
                page,
                &Selection::default(),
                None,
                &status,
                &theme::DARK,
            ));
        });
        out.unwrap()
    }

    #[test]
    fn canvas_rect_excludes_panels() {
        let (doc, page) = doc_with(3);
        let (_, canvas) = frame(&mut Panels::default(), &doc, page, false);
        assert!(
            canvas.left() >= 180.0 && canvas.right() <= 1280.0 - 240.0,
            "{canvas:?}"
        );
        assert!(
            canvas.top() >= 46.0 && canvas.bottom() <= 800.0 - 26.0,
            "{canvas:?}"
        );
    }

    #[test]
    fn home_screen_has_no_canvas() {
        let (doc, page) = doc_with(1);
        let mut panels = Panels::default();
        let (_, canvas) = frame(&mut panels, &doc, page, true);
        assert!(canvas.is_negative() || canvas.area() == 0.0, "{canvas:?}");
        // Dev Mode renders code for the page without panicking.
        panels.dev_mode = true;
        let _ = frame(&mut panels, &doc, page, false);
    }

    #[test]
    fn layers_flatten_respects_expansion() {
        let (mut doc, page) = doc_with(2);
        let first = doc.children(page)[0];
        doc.build_child(first, Node::new(NodeKind::Rectangle, Symbol::EMPTY))
            .unwrap();
        let mut l = LayersPanel::default();
        l.rebuild(&doc, page);
        assert_eq!(l.rows().len(), 2, "collapsed by default");
        l.toggle(first);
        l.rebuild(&doc, page);
        assert_eq!(l.rows().len(), 3);
        // Top of the stack first: the second layer, then the first and its child.
        assert_eq!(l.rows()[1].0, first);
        assert_eq!(l.rows()[2].1, 1, "child is indented");
    }
}
