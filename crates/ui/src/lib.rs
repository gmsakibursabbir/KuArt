//! Editor UI drawn with egui around the canvas, Figma-style: a top bar with the KuArt menu,
//! tools and actions; pages and layers on the left; Design and Dev tabs on the right; a status
//! bar; and a home screen with recent files. Panels read the document and return [`Action`]s;
//! the app applies them through history.

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
    Undo,
    Redo,
    ZoomToFit,
    ZoomIn,
    ZoomOut,
    Zoom100,
    ToggleDiagnostics,
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
}

#[derive(Default)]
pub struct Panels {
    pub theme: ThemeChoice,
    pub layers: LayersPanel,
    /// Right panel shows Dev Mode (code and inspect) instead of Design.
    pub dev_mode: bool,
    inspector: Inspector,
    dev: devmode::DevMode,
    home: home::Home,
}

const TOOL_ICONS: [(Tool, &str); 8] = [
    (Tool::Move, icons::MOUSE_POINTER_2),
    (Tool::Frame, icons::FRAME),
    (Tool::Rectangle, icons::SQUARE),
    (Tool::Ellipse, icons::CIRCLE),
    (Tool::Polygon, icons::TRIANGLE),
    (Tool::Star, icons::STAR),
    (Tool::Line, icons::SLASH),
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
                let mut tab = usize::from(self.dev_mode);
                segmented(ui, t, &["Design", "Dev Mode"], &mut tab);
                self.dev_mode = tab == 1;
                egui::ScrollArea::vertical()
                    .id_salt("right_scroll")
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        if self.dev_mode {
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
                    ui.add_space(6.0);
                    ui.label(RichText::new("KuArt").strong().color(t.text_primary));

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
                                menu_item(ui, "Zoom in", "Ctrl+=", actions, Action::ZoomIn);
                                menu_item(ui, "Zoom out", "Ctrl+-", actions, Action::ZoomOut);
                                menu_item(ui, "Zoom to fit", "Shift+1", actions, Action::ZoomToFit);
                                menu_item(ui, "Zoom to 100%", "Ctrl+0", actions, Action::Zoom100);
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

    fn main_menu(
        &mut self,
        ui: &mut Ui,
        status: &Status<'_>,
        t: &Tokens,
        actions: &mut Vec<Action>,
    ) {
        let logo = RichText::new(icons::MENU)
            .family(egui::FontFamily::Name(theme::ICONS.into()))
            .size(16.0)
            .color(t.text_primary);
        ui.menu_button(logo, |ui| {
            ui.set_min_width(230.0);
            menu_item(ui, "Back to files", "", actions, Action::Home);
            ui.separator();
            menu_item(ui, "New design", "Ctrl+N", actions, Action::New);
            menu_item(ui, "Open…", "Ctrl+O", actions, Action::Open);
            menu_item(ui, "Import…", "", actions, Action::Import);
            ui.menu_button("Open recent", |ui| {
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
            menu_item(ui, "Save", "Ctrl+S", actions, Action::Save);
            menu_item(ui, "Save as…", "Ctrl+Shift+S", actions, Action::SaveAs);
            menu_item(ui, "Export…", "Ctrl+Shift+E", actions, Action::Export);
            ui.separator();
            ui.menu_button("Edit", |ui| {
                menu_item(ui, "Undo", "Ctrl+Z", actions, Action::Undo);
                menu_item(ui, "Redo", "Ctrl+Y", actions, Action::Redo);
                ui.separator();
                menu_item(ui, "Copy", "Ctrl+C", actions, Action::Edit(Edit::Copy));
                menu_item(ui, "Cut", "Ctrl+X", actions, Action::Edit(Edit::Cut));
                menu_item(ui, "Paste", "Ctrl+V", actions, Action::Edit(Edit::Paste));
                menu_item(
                    ui,
                    "Duplicate",
                    "Ctrl+D",
                    actions,
                    Action::Edit(Edit::Duplicate),
                );
                menu_item(ui, "Delete", "Del", actions, Action::Edit(Edit::Delete));
                ui.separator();
                menu_item(
                    ui,
                    "Select all",
                    "Ctrl+A",
                    actions,
                    Action::Edit(Edit::SelectAll),
                );
            });
            ui.menu_button("Object", |ui| {
                menu_item(ui, "Group", "Ctrl+G", actions, Action::Edit(Edit::Group));
                menu_item(
                    ui,
                    "Ungroup",
                    "Ctrl+Shift+G",
                    actions,
                    Action::Edit(Edit::Ungroup),
                );
                ui.separator();
                menu_item(
                    ui,
                    "Bring forward",
                    "Ctrl+]",
                    actions,
                    Action::Edit(Edit::BringForward),
                );
                menu_item(
                    ui,
                    "Send backward",
                    "Ctrl+[",
                    actions,
                    Action::Edit(Edit::SendBackward),
                );
            });
            ui.menu_button("View", |ui| {
                menu_item(ui, "Zoom to fit", "Shift+1", actions, Action::ZoomToFit);
                menu_item(ui, "Zoom to 100%", "Ctrl+0", actions, Action::Zoom100);
                ui.separator();
                if ui.checkbox(&mut self.dev_mode, "Dev Mode").clicked() {
                    ui.close();
                }
                menu_item(ui, "Diagnostics", "F12", actions, Action::ToggleDiagnostics);
                ui.separator();
                for c in ThemeChoice::ALL {
                    if ui.radio(self.theme == c, c.label()).clicked() {
                        self.theme = c;
                        ui.close();
                    }
                }
            });
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
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(status.renderer).size(11.0).color(t.text_muted));
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

fn menu_item(ui: &mut Ui, label: &str, shortcut: &str, actions: &mut Vec<Action>, action: Action) {
    let clicked = ui
        .horizontal(|ui| {
            let r = ui.add(
                egui::Button::new(label)
                    .frame(false)
                    .min_size(egui::vec2(170.0, 0.0)),
            );
            if !shortcut.is_empty() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(shortcut).size(11.0).weak());
                });
            }
            r.clicked()
        })
        .inner;
    if clicked {
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
                            let tip = format!("{}  {}", tool.label(), tool.shortcut());
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
