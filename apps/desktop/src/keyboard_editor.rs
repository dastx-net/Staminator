//! Standalone editor widget for user keyboard geometry.
//!
//! Add `mod keyboard_editor;` and construct with `KeyboardEditor::new(schema)`.
//! Call `show(ctx)` each frame. It returns a validated schema clone when the
//! user chooses Save; the caller owns persistence and should use `save_new`.

use crate::localization::tr;
use egui::{Color32, Context, Pos2, Rect, Sense, Stroke, Vec2};
use staminator_keyboard_schema::{
    Canvas, Finger, Geometry, Key, KeyboardSchema, Point, SCHEMA_VERSION,
};
use std::array;

pub struct KeyboardEditor {
    schema: KeyboardSchema,
    selected: Option<usize>,
    error: Option<String>,
    polygon_text: String,
    open: bool,
    label_states: [LabelState; 2],
    canvas_drag: Option<CanvasDrag>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CanvasDragMode {
    MoveRect,
    ResizeRect,
    MovePolygon,
    PolygonVertex(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CanvasDrag {
    key_index: usize,
    mode: CanvasDragMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LabelMode {
    Automatic,
    Blank,
    Custom,
}

#[derive(Clone, Debug)]
struct LabelState {
    mode: LabelMode,
    draft: String,
}
impl LabelState {
    fn value(&self) -> Option<String> {
        match self.mode {
            LabelMode::Automatic => None,
            LabelMode::Blank => Some(String::new()),
            LabelMode::Custom => Some(self.draft.clone()),
        }
    }
}

impl KeyboardEditor {
    pub fn new(schema: KeyboardSchema) -> Self {
        let label_states = array::from_fn(|index| {
            let value = schema.keys.first().and_then(|key| {
                if index == 0 {
                    key.label.as_ref()
                } else {
                    key.shifted_label.as_ref()
                }
            });
            match value {
                None => LabelState {
                    mode: LabelMode::Automatic,
                    draft: String::new(),
                },
                Some(value) if value.is_empty() => LabelState {
                    mode: LabelMode::Blank,
                    draft: String::new(),
                },
                Some(value) => LabelState {
                    mode: LabelMode::Custom,
                    draft: value.clone(),
                },
            }
        });
        let mut this = Self {
            schema,
            selected: None,
            error: None,
            polygon_text: String::new(),
            open: true,
            label_states,
            canvas_drag: None,
        };
        this.sync_polygon_text();
        this
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Draws the editor and emits at most one action. No filesystem access occurs.
    pub fn show(&mut self, ctx: &Context) -> Option<KeyboardSchema> {
        let mut result = None;
        if !self.open {
            return None;
        }
        let mut window_open = self.open;
        let mut close_clicked = false;
        egui::Window::new(tr("keyboard_editor.window"))
            .open(&mut window_open)
            .default_size(Vec2::new(980.0, 680.0))
            .show(ctx, |ui| {
                crate::ui_style::apply(ui);
                let footer_height = ui.spacing().interact_size.y + 24.0;
                egui::ScrollArea::vertical()
                    .max_height((ui.available_height() - footer_height).max(0.0))
                    .show(ui, |ui| {
                        ui.heading(tr("keyboard_editor.heading"));
                        ui.label(tr("keyboard_editor.intro"));
                        ui.add_space(8.0);
                        egui::Grid::new("keyboard_editor_metadata")
                            .num_columns(2)
                            .spacing([12.0, 8.0])
                            .show(ui, |ui| {
                                ui.label(tr("keyboard_editor.layout_name"));
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.schema.name)
                                        .desired_width(260.0),
                                );
                                ui.end_row();
                                ui.label(tr("keyboard_editor.canvas_size"));
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::DragValue::new(&mut self.schema.canvas.width)
                                            .prefix(format!(
                                                "{} ",
                                                tr("keyboard_editor.width_short")
                                            ))
                                            .range(1.0..=4096.0),
                                    );
                                    ui.add(
                                        egui::DragValue::new(&mut self.schema.canvas.height)
                                            .prefix(format!(
                                                "{} ",
                                                tr("keyboard_editor.height_short")
                                            ))
                                            .range(1.0..=4096.0),
                                    );
                                });
                                ui.end_row();
                            });
                        ui.checkbox(
                            &mut self.schema.show_finger_colors,
                            tr("keyboard_editor.finger_colors"),
                        );
                        ui.add_space(12.0);
                        ui.heading(tr("keyboard_editor.keys"));
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .button(format!("＋ {}", tr("keyboard_editor.add_key")))
                                .clicked()
                            {
                                self.add_key();
                            }
                            let delete = ui.add_enabled(
                                self.selected.is_some(),
                                egui::Button::new(tr("keyboard_editor.delete_selected")),
                            );
                            if delete.clicked() {
                                self.delete_selected();
                            }
                        });
                        ui.add_space(4.0);
                        let narrow = ui.available_width() < 680.0;
                        if narrow {
                            self.key_list_ui(ui, narrow);
                            ui.add_space(8.0);
                            ui.heading(tr("keyboard_editor.preview"));
                            ui.label(tr("keyboard_editor.drag_help"));
                            self.canvas_ui(ui);
                        } else {
                            ui.horizontal(|ui| {
                                self.key_list_ui(ui, narrow);
                                ui.separator();
                                ui.vertical(|ui| {
                                    ui.heading(tr("keyboard_editor.preview"));
                                    ui.label(tr("keyboard_editor.drag_help"));
                                    self.canvas_ui(ui);
                                });
                            });
                        }
                        if let Some(i) = self.selected {
                            ui.add_space(12.0);
                            ui.heading(tr("keyboard_editor.selected_key"));
                            self.properties_ui(ui, i);
                        }
                        if let Some(error) = &self.error {
                            ui.colored_label(ui.visuals().error_fg_color, error);
                        }
                    });
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr("keyboard_editor.close")).clicked() {
                        close_clicked = true;
                    }
                    if ui.button(tr("keyboard_editor.save_copy")).clicked() {
                        result = self.validated_copy();
                    }
                });
            });
        self.open = window_open && !close_clicked;
        result
    }

    fn validated_copy(&mut self) -> Option<KeyboardSchema> {
        match self.schema.validate() {
            Ok(()) => {
                self.error = None;
                Some(self.schema.clone())
            }
            Err(error) => {
                self.error = Some(error.to_string());
                None
            }
        }
    }

    fn add_key(&mut self) {
        if self.schema.keys.len() >= 256 {
            self.error = Some(tr("keyboard_editor.max_keys"));
            return;
        }
        // Key size follows the canvas proportions. The default 16 × 5 board
        // therefore gets roughly square keys instead of oversized pixels.
        let width = (self.schema.canvas.width / 16.0).max(0.1);
        let height = (self.schema.canvas.height / 5.0).max(0.1);
        let columns = (self.schema.canvas.width / width).floor().max(1.0) as usize;
        let slot = self.schema.keys.len();
        let x = (slot % columns) as f64 * width;
        let y = (slot / columns) as f64 * height;
        let key = Key {
            physical_id: String::new(),
            geometry: Geometry::Rect {
                x: x.min((self.schema.canvas.width - width).max(0.0)),
                y: y.min((self.schema.canvas.height - height).max(0.0)),
                width,
                height,
            },
            label: None,
            shifted_label: None,
            color: None,
            finger: None,
        };
        self.schema.keys.push(key);
        self.select(self.schema.keys.len() - 1);
    }
    fn delete_selected(&mut self) {
        if let Some(i) = self.selected.take() {
            self.schema.keys.remove(i);
        }
        self.sync_polygon_text();
        self.error = None;
    }
    fn select(&mut self, index: usize) {
        self.selected = Some(index);
        self.sync_polygon_text();
        if let Some(key) = self.schema.keys.get(index) {
            self.label_states[0] = label_state(key.label.as_ref());
            self.label_states[1] = label_state(key.shifted_label.as_ref());
        }
    }
    fn sync_polygon_text(&mut self) {
        self.polygon_text = self
            .selected
            .and_then(|i| self.schema.keys.get(i))
            .and_then(|k| match &k.geometry {
                Geometry::Polygon { points } => Some(
                    points
                        .iter()
                        .map(|p| format!("{},{}", p.x, p.y))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                _ => None,
            })
            .unwrap_or_default();
    }

    fn key_list_ui(&mut self, ui: &mut egui::Ui, narrow: bool) {
        ui.vertical(|ui| {
            ui.set_min_width(if narrow { 0.0 } else { 180.0 });
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    let titles: Vec<String> = self
                        .schema
                        .keys
                        .iter()
                        .map(|key| {
                            if key.physical_id.is_empty() {
                                tr("keyboard_editor.new_key")
                            } else {
                                key.physical_id.clone()
                            }
                        })
                        .collect();
                    let mut clicked = None;
                    for (i, title) in titles.iter().enumerate() {
                        let label = if self.selected == Some(i) {
                            format!("✓ {}  {title}", tr("keyboard_editor.selected"))
                        } else {
                            format!("○  {title}")
                        };
                        if ui
                            .selectable_label(self.selected == Some(i), label)
                            .clicked()
                        {
                            clicked = Some(i);
                        }
                    }
                    if let Some(i) = clicked {
                        self.select(i);
                    }
                });
        });
    }
    fn canvas_ui(&mut self, ui: &mut egui::Ui) {
        let canvas = self.schema.canvas;
        let avail = ui
            .available_size()
            .min(Vec2::new(740.0, 360.0))
            .max(Vec2::new(200.0, 120.0));
        let scale = (avail.x / canvas.width as f32)
            .min(avail.y / canvas.height as f32)
            .max(0.01);
        let size = Vec2::new(canvas.width as f32 * scale, canvas.height as f32 * scale);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        ui.painter().rect_filled(rect, 2.0, Color32::from_gray(32));
        ui.painter().rect_stroke(
            rect,
            2.0,
            Stroke::new(1.0, Color32::GRAY),
            egui::StrokeKind::Inside,
        );
        let origin = rect.min;
        for (i, key) in self.schema.keys.iter_mut().enumerate() {
            if let Geometry::Rect {
                x,
                y,
                width,
                height,
            } = &mut key.geometry
            {
                let key_rect = Rect::from_min_size(
                    origin + Vec2::new(*x as f32, *y as f32) * scale,
                    Vec2::new(*width as f32, *height as f32) * scale,
                );
                let id = ui.make_persistent_id(("keyboard_key", i));
                let response = ui.interact(key_rect, id, Sense::click_and_drag());
                let fill = key
                    .color
                    .as_deref()
                    .and_then(parse_color)
                    .unwrap_or(Color32::from_rgb(81, 102, 120));
                ui.painter().rect_filled(key_rect, 3.0, fill);
                ui.painter().rect_stroke(
                    key_rect,
                    3.0,
                    Stroke::new(
                        if self.selected == Some(i) { 2.0 } else { 1.0 },
                        if self.selected == Some(i) {
                            Color32::YELLOW
                        } else {
                            Color32::LIGHT_GRAY
                        },
                    ),
                    egui::StrokeKind::Inside,
                );
                let label = key.label.as_deref().unwrap_or(&key.physical_id);
                ui.painter().text(
                    key_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::FontId::proportional(13.0),
                    Color32::WHITE,
                );
                if response.clicked() {
                    self.selected = Some(i);
                    self.label_states[0] = label_state(key.label.as_ref());
                    self.label_states[1] = label_state(key.shifted_label.as_ref());
                }
                if response.drag_started() {
                    let press = response.ctx.input(|input| input.pointer.press_origin());
                    let mode = if press.is_some_and(|p| key_rect.right_bottom().distance(p) < 14.0)
                    {
                        CanvasDragMode::ResizeRect
                    } else {
                        CanvasDragMode::MoveRect
                    };
                    self.canvas_drag = Some(CanvasDrag { key_index: i, mode });
                }
                if response.dragged() {
                    if self.selected != Some(i) {
                        self.selected = Some(i);
                        self.label_states[0] = label_state(key.label.as_ref());
                        self.label_states[1] = label_state(key.shifted_label.as_ref());
                        self.polygon_text.clear();
                    }
                    let delta = response.drag_delta() / scale;
                    if response
                        .ctx
                        .input(|input| input.pointer.button_down(egui::PointerButton::Primary))
                        && response.dragged_by(egui::PointerButton::Primary)
                        && let Some(drag) = self.canvas_drag.filter(|drag| drag.key_index == i)
                    {
                        match drag.mode {
                            CanvasDragMode::ResizeRect => {
                                let max_width = (canvas.width - *x).max(0.0);
                                let max_height = (canvas.height - *y).max(0.0);
                                let min_width = (canvas.width / 100.0).min(max_width);
                                let min_height = (canvas.height / 50.0).min(max_height);
                                *width = (*width + delta.x as f64).clamp(min_width, max_width);
                                *height = (*height + delta.y as f64).clamp(min_height, max_height);
                            }
                            CanvasDragMode::MoveRect => {
                                *x = (*x + delta.x as f64)
                                    .clamp(0.0, (canvas.width - *width).max(0.0));
                                *y = (*y + delta.y as f64)
                                    .clamp(0.0, (canvas.height - *height).max(0.0));
                            }
                            _ => {}
                        }
                    }
                }
                if response.drag_stopped() {
                    self.canvas_drag = None;
                }
                ui.painter()
                    .circle_filled(key_rect.right_bottom(), 4.0, Color32::YELLOW);
            } else if let Geometry::Polygon { points } = &mut key.geometry
                && points.len() >= 3
            {
                let pts: Vec<Pos2> = points
                    .iter()
                    .map(|p| origin + Vec2::new(p.x as f32, p.y as f32) * scale)
                    .collect();
                let bounds = pts.iter().fold(Rect::NOTHING, |r, p| {
                    r.union(Rect::from_center_size(*p, Vec2::splat(0.1)))
                });
                let response = ui.interact(
                    bounds,
                    ui.make_persistent_id(("keyboard_polygon", i)),
                    Sense::click_and_drag(),
                );
                let fill = key
                    .color
                    .as_deref()
                    .and_then(parse_color)
                    .unwrap_or(Color32::from_rgb(81, 102, 120));
                ui.painter()
                    .add(crate::keyboard::polygon_fill_mesh(&pts, fill));
                ui.painter().add(egui::Shape::Path(egui::epaint::PathShape {
                    points: pts.clone(),
                    closed: true,
                    fill: Color32::TRANSPARENT,
                    stroke: Stroke::new(
                        if self.selected == Some(i) { 2.0 } else { 1.0 },
                        if self.selected == Some(i) {
                            Color32::YELLOW
                        } else {
                            Color32::LIGHT_GRAY
                        },
                    )
                    .into(),
                }));
                for p in &pts {
                    ui.painter().circle_filled(*p, 3.5, Color32::YELLOW);
                }
                if response.clicked() {
                    self.selected = Some(i);
                    self.label_states[0] = label_state(key.label.as_ref());
                    self.label_states[1] = label_state(key.shifted_label.as_ref());
                    self.polygon_text = points
                        .iter()
                        .map(|p| format!("{},{}", p.x, p.y))
                        .collect::<Vec<_>>()
                        .join("\n");
                }
                if response.drag_started() {
                    let press = response.ctx.input(|input| input.pointer.press_origin());
                    let nearest = press.and_then(|p| {
                        pts.iter()
                            .enumerate()
                            .min_by(|(_, a), (_, b)| a.distance(p).total_cmp(&b.distance(p)))
                            .map(|(j, _)| j)
                    });
                    let mode = nearest
                        .filter(|j| press.is_some_and(|p| pts[*j].distance(p) < 14.0))
                        .map(CanvasDragMode::PolygonVertex)
                        .unwrap_or(CanvasDragMode::MovePolygon);
                    self.canvas_drag = Some(CanvasDrag { key_index: i, mode });
                }
                if response.dragged() {
                    if self.selected != Some(i) {
                        self.selected = Some(i);
                        self.label_states[0] = label_state(key.label.as_ref());
                        self.label_states[1] = label_state(key.shifted_label.as_ref());
                        self.polygon_text = points
                            .iter()
                            .map(|p| format!("{},{}", p.x, p.y))
                            .collect::<Vec<_>>()
                            .join("\n");
                    }
                    let delta = response.drag_delta() / scale;
                    if let Some(drag) = self.canvas_drag.filter(|drag| drag.key_index == i) {
                        match drag.mode {
                            CanvasDragMode::PolygonVertex(j) => {
                                points[j].x =
                                    (points[j].x + delta.x as f64).clamp(0.0, canvas.width);
                                points[j].y =
                                    (points[j].y + delta.y as f64).clamp(0.0, canvas.height);
                            }
                            CanvasDragMode::MovePolygon => {
                                let min_x =
                                    points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
                                let min_y =
                                    points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
                                let max_x =
                                    points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
                                let max_y =
                                    points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
                                if min_x.is_finite()
                                    && max_x.is_finite()
                                    && min_y.is_finite()
                                    && max_y.is_finite()
                                    && min_x <= max_x
                                    && min_y <= max_y
                                    && -min_x <= canvas.width - max_x
                                    && -min_y <= canvas.height - max_y
                                {
                                    let dx = (delta.x as f64).clamp(-min_x, canvas.width - max_x);
                                    let dy = (delta.y as f64).clamp(-min_y, canvas.height - max_y);
                                    for point in points.iter_mut() {
                                        point.x += dx;
                                        point.y += dy;
                                    }
                                }
                            }
                            _ => {}
                        }
                        self.polygon_text = points
                            .iter()
                            .map(|p| format!("{},{}", p.x, p.y))
                            .collect::<Vec<_>>()
                            .join("\n");
                    }
                }
                if response.drag_stopped() {
                    self.canvas_drag = None;
                }
            }
        }
    }

    fn properties_ui(&mut self, ui: &mut egui::Ui, i: usize) {
        let canvas = self.schema.canvas;
        let label = label_editor(ui, "keyboard_editor.label", &mut self.label_states[0]);
        let shifted_label = label_editor(ui, "keyboard_editor.shift", &mut self.label_states[1]);
        let key = &mut self.schema.keys[i];
        ui.horizontal(|ui| {
            ui.label(tr("keyboard_editor.physical_id"));
            ui.text_edit_singleline(&mut key.physical_id);
            ui.label(tr("keyboard_editor.color"));
            optional_color(ui, &mut key.color);
        });
        key.label = label;
        key.shifted_label = shifted_label;
        ui.horizontal(|ui| {
            ui.label(tr("keyboard_editor.finger"));
            let mut finger = key.finger;
            egui::ComboBox::from_id_salt("finger_combo")
                .selected_text(
                    finger
                        .map(finger_name)
                        .unwrap_or_else(|| tr("keyboard_editor.none")),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut finger, None, tr("keyboard_editor.none"));
                    for f in fingers() {
                        ui.selectable_value(&mut finger, Some(f), finger_name(f));
                    }
                });
            key.finger = finger;
            ui.label(tr("keyboard_editor.geometry"));
            let poly = matches!(key.geometry, Geometry::Polygon { .. });
            let mut is_poly = poly;
            if ui
                .checkbox(&mut is_poly, tr("keyboard_editor.polygon"))
                .changed()
            {
                key.geometry = toggle_geometry(&key.geometry, canvas, is_poly);
                self.polygon_text = if let Geometry::Polygon { points } = &key.geometry {
                    points
                        .iter()
                        .map(|p| format!("{},{}", p.x, p.y))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    String::new()
                };
            }
            if let Geometry::Rect {
                x,
                y,
                width,
                height,
            } = &mut key.geometry
            {
                ui.add(
                    egui::DragValue::new(x)
                        .prefix(format!("{} ", tr("keyboard_editor.x_short")))
                        .range(0.0..=4096.0),
                );
                ui.add(
                    egui::DragValue::new(y)
                        .prefix(format!("{} ", tr("keyboard_editor.y_short")))
                        .range(0.0..=4096.0),
                );
                ui.add(
                    egui::DragValue::new(width)
                        .prefix(format!("{} ", tr("keyboard_editor.width_short")))
                        .range((canvas.width / 100.0).max(0.01)..=canvas.width),
                );
                ui.add(
                    egui::DragValue::new(height)
                        .prefix(format!("{} ", tr("keyboard_editor.height_short")))
                        .range((canvas.height / 50.0).max(0.01)..=canvas.height),
                );
            }
        });
        if let Geometry::Polygon { points } = &mut key.geometry {
            ui.label(tr("keyboard_editor.polygon_vertices"));
            if ui
                .add(
                    egui::TextEdit::multiline(&mut self.polygon_text)
                        .desired_rows(3)
                        .desired_width(500.0),
                )
                .changed()
            {
                apply_polygon_text(points, &self.polygon_text);
            }
        }
    }
}

fn label_state(value: Option<&String>) -> LabelState {
    match value {
        None => LabelState {
            mode: LabelMode::Automatic,
            draft: String::new(),
        },
        Some(value) if value.is_empty() => LabelState {
            mode: LabelMode::Blank,
            draft: String::new(),
        },
        Some(value) => LabelState {
            mode: LabelMode::Custom,
            draft: value.clone(),
        },
    }
}

fn toggle_geometry(geometry: &Geometry, canvas: Canvas, polygon: bool) -> Geometry {
    let (x, y, width, height) = match geometry {
        Geometry::Rect {
            x,
            y,
            width,
            height,
        } => (*x, *y, *width, *height),
        Geometry::Polygon { points } => {
            if points.len() < 3 {
                return Geometry::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: (canvas.width / 16.0).max(0.1).min(canvas.width),
                    height: (canvas.height / 5.0).max(0.1).min(canvas.height),
                };
            }
            let min_x = points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
            let min_y = points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
            let max_x = points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
            let max_y = points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
            (min_x, min_y, max_x - min_x, max_y - min_y)
        }
    };
    let width = width.max(canvas.width / 100.0).min(canvas.width);
    let height = height.max(canvas.height / 50.0).min(canvas.height);
    let x = x.clamp(0.0, (canvas.width - width).max(0.0));
    let y = y.clamp(0.0, (canvas.height - height).max(0.0));
    if polygon {
        Geometry::Polygon {
            points: vec![
                Point { x, y },
                Point { x: x + width, y },
                Point {
                    x: x + width,
                    y: y + height,
                },
                Point { x, y: y + height },
            ],
        }
    } else {
        Geometry::Rect {
            x,
            y,
            width,
            height,
        }
    }
}

fn parse_polygon_points(text: &str) -> Result<Vec<Point>, ()> {
    let points: Vec<Point> = text
        .lines()
        .map(|line| {
            let (x, y) = line.split_once(',').ok_or(())?;
            Ok(Point {
                x: x.trim().parse().map_err(|_| ())?,
                y: y.trim().parse().map_err(|_| ())?,
            })
        })
        .collect::<Result<_, _>>()?;
    if points.len() < 3 {
        return Err(());
    }
    Ok(points)
}

fn apply_polygon_text(points: &mut Vec<Point>, text: &str) -> bool {
    match parse_polygon_points(text) {
        Ok(parsed) => {
            *points = parsed;
            true
        }
        Err(()) => false,
    }
}

fn label_editor(ui: &mut egui::Ui, title_key: &str, state: &mut LabelState) -> Option<String> {
    let title = tr(title_key);
    ui.horizontal(|ui| {
        ui.label(&title);
        egui::ComboBox::from_id_salt(("label_mode", title_key))
            .selected_text(match state.mode {
                LabelMode::Automatic => tr("keyboard_editor.label_auto"),
                LabelMode::Blank => tr("keyboard_editor.label_blank"),
                LabelMode::Custom => tr("keyboard_editor.label_custom"),
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut state.mode,
                    LabelMode::Automatic,
                    tr("keyboard_editor.label_auto"),
                );
                ui.selectable_value(
                    &mut state.mode,
                    LabelMode::Blank,
                    tr("keyboard_editor.label_blank"),
                );
                ui.selectable_value(
                    &mut state.mode,
                    LabelMode::Custom,
                    tr("keyboard_editor.label_custom"),
                );
            });
        if state.mode == LabelMode::Custom {
            ui.add(egui::TextEdit::singleline(&mut state.draft).desired_width(120.0));
        }
    });
    state.value()
}
fn optional_color(ui: &mut egui::Ui, value: &mut Option<String>) {
    let mut text = value.clone().unwrap_or_default();
    if ui
        .add(egui::TextEdit::singleline(&mut text).desired_width(90.0))
        .changed()
    {
        *value = (!text.is_empty()).then_some(text);
    }
}
fn parse_color(s: &str) -> Option<Color32> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color32::from_rgba_unmultiplied(
        r,
        g,
        b,
        if hex.len() == 8 {
            u8::from_str_radix(&hex[6..8], 16).ok()?
        } else {
            255
        },
    ))
}
fn fingers() -> [Finger; 10] {
    [
        Finger::LeftLittle,
        Finger::LeftRing,
        Finger::LeftMiddle,
        Finger::LeftIndex,
        Finger::LeftThumb,
        Finger::RightThumb,
        Finger::RightIndex,
        Finger::RightMiddle,
        Finger::RightRing,
        Finger::RightLittle,
    ]
}
fn finger_name(f: Finger) -> String {
    match f {
        Finger::LeftLittle => tr("keyboard_manager.left_little"),
        Finger::LeftRing => tr("keyboard_manager.left_ring"),
        Finger::LeftMiddle => tr("keyboard_manager.left_middle"),
        Finger::LeftIndex => tr("keyboard_manager.left_index"),
        Finger::LeftThumb => tr("keyboard_manager.left_thumb"),
        Finger::RightThumb => tr("keyboard_manager.right_thumb"),
        Finger::RightIndex => tr("keyboard_manager.right_index"),
        Finger::RightMiddle => tr("keyboard_manager.right_middle"),
        Finger::RightRing => tr("keyboard_manager.right_ring"),
        Finger::RightLittle => tr("keyboard_manager.right_little"),
    }
}

/// Empty schema suitable as a starting point for a new layout.
pub fn blank_schema(name: impl Into<String>, width: f64, height: f64) -> KeyboardSchema {
    KeyboardSchema {
        show_finger_colors: true,
        style: None,
        version: SCHEMA_VERSION,
        name: name.into(),
        canvas: Canvas { width, height },
        keys: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas_frame(
        ctx: &Context,
        editor: &mut KeyboardEditor,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                editor.canvas_ui(ui);
            },
        );
        output.textures_delta.clear();
        output
    }

    fn key_list_frame(
        ctx: &Context,
        editor: &mut KeyboardEditor,
        narrow: bool,
        width: f32,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                ui.set_width(width);
                editor.key_list_ui(ui, narrow);
            },
        );
        output.textures_delta.clear();
        output
    }

    fn editor_window_frame(
        ctx: &Context,
        editor: &mut KeyboardEditor,
        size: Vec2,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<KeyboardSchema>) {
        let mut result = None;
        let mut output = ctx.run_pass(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ctx| {
                ctx.root_ui(|_| {
                    result = editor.show(ctx);
                });
            },
        );
        output.textures_delta.clear();
        (output, result)
    }

    fn properties_frame(ctx: &Context, editor: &mut KeyboardEditor, index: usize) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0))),
                ..Default::default()
            },
            |ui| editor.properties_ui(ui, index),
        );
        output.textures_delta.clear();
    }

    fn accessible_properties_frame(
        ctx: &Context,
        editor: &mut KeyboardEditor,
        index: usize,
        events: Vec<egui::Event>,
    ) -> Vec<(egui::accesskit::NodeId, egui::accesskit::Node)> {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| editor.properties_ui(ui, index),
        );
        output.textures_delta.clear();
        output
            .platform_output
            .accesskit_update
            .take()
            .map(|update| update.nodes)
            .unwrap_or_default()
    }

    fn test_key(id: &str, geometry: Geometry) -> Key {
        Key {
            physical_id: id.into(),
            geometry,
            label: Some("a".into()),
            shifted_label: Some("A".into()),
            color: None,
            finger: None,
        }
    }
    #[test]
    fn blank_schema_is_valid_and_round_trips_as_a_new_copy_value() {
        let schema = blank_schema("Custom", 300.0, 100.0);
        schema.validate().unwrap();
        let _editor = KeyboardEditor::new(schema);
    }

    #[test]
    fn editor_labels_follow_interface_language() {
        crate::localization::set_language("EN");
        assert_eq!(tr("keyboard_editor.window"), "Keyboard layout editor");
        assert_eq!(tr("keyboard_editor.polygon"), "Polygon");
        assert_eq!(finger_name(Finger::LeftLittle), "Left little");

        crate::localization::set_language("RU");
        assert_eq!(tr("keyboard_editor.window"), "Редактор клавиатуры");
        assert_eq!(tr("keyboard_editor.polygon"), "Многоугольник");
        assert_eq!(finger_name(Finger::LeftLittle), "Левый мизинец");

        crate::localization::set_language("DE");
        assert_eq!(tr("keyboard_editor.window"), "Tastaturlayout-Editor");
        crate::localization::set_language("RU");
    }

    #[test]
    fn added_key_fits_small_unit_canvas_and_layout_remains_valid() {
        let mut editor = KeyboardEditor::new(blank_schema("Compact", 16.0, 5.0));
        editor.add_key();
        editor.schema.keys[0].physical_id = "KeyA".into();
        let Geometry::Rect {
            x,
            y,
            width,
            height,
        } = &editor.schema.keys[0].geometry
        else {
            panic!("new key should be rectangular")
        };
        assert_eq!((*x, *y, *width, *height), (0.0, 0.0, 1.0, 1.0));
        editor.schema.validate().unwrap();
    }

    #[test]
    fn key_list_selects_the_clicked_key_in_narrow_and_wide_layouts() {
        for (narrow, width) in [(true, 120.0), (false, 240.0)] {
            let mut schema = blank_schema("Board", 16.0, 5.0);
            schema.keys.push(test_key(
                "KeyA",
                Geometry::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                },
            ));
            schema.keys.push(test_key(
                "KeyB",
                Geometry::Rect {
                    x: 1.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                },
            ));
            let mut editor = KeyboardEditor::new(schema);
            editor.select(0);
            let ctx = Context::default();
            let frame = key_list_frame(&ctx, &mut editor, narrow, width, vec![]);
            let target = frame
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == "○  KeyB" => {
                        Some(Rect::from_min_size(text.pos, text.galley.size()).center())
                    }
                    _ => None,
                })
                .expect("second key should be visible in the key list");

            key_list_frame(
                &ctx,
                &mut editor,
                narrow,
                width,
                vec![
                    egui::Event::PointerMoved(target),
                    egui::Event::PointerButton {
                        pos: target,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            key_list_frame(
                &ctx,
                &mut editor,
                narrow,
                width,
                vec![egui::Event::PointerButton {
                    pos: target,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert_eq!(
                editor.selected,
                Some(1),
                "clicking the second key should select it (narrow={narrow})"
            );
        }
    }

    #[test]
    fn save_action_stays_visible_and_works_in_short_window_with_selected_key() {
        crate::localization::set_language("EN");
        let mut schema = blank_schema("Board", 16.0, 5.0);
        schema.keys.push(test_key(
            "KeyA",
            Geometry::Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
        ));
        let mut editor = KeyboardEditor::new(schema);
        editor.select(0);
        let ctx = Context::default();
        ctx.enable_accesskit();
        let size = Vec2::new(600.0, 360.0);
        let mut save_bounds = None;
        for _ in 0..3 {
            let (frame, result) = editor_window_frame(&ctx, &mut editor, size, vec![]);
            assert!(result.is_none());
            save_bounds = frame
                .platform_output
                .accesskit_update
                .as_ref()
                .and_then(|update| {
                    update.nodes.iter().find_map(|(_, node)| {
                        (node.role() == egui::accesskit::Role::Button
                            && node.label() == Some("Validate & save copy"))
                        .then(|| node.bounds())
                        .flatten()
                    })
                })
                .or(save_bounds);
        }
        let save_bounds = save_bounds.expect("save button should be accessible");
        let save_rect = Rect::from_min_max(
            Pos2::new(save_bounds.x0 as f32, save_bounds.y0 as f32),
            Pos2::new(save_bounds.x1 as f32, save_bounds.y1 as f32),
        );
        let window_rect = ctx
            .memory(|memory| {
                memory
                    .areas()
                    .visible_layer_ids()
                    .into_iter()
                    .filter(|layer| layer.order == egui::Order::Middle)
                    .find_map(|layer| memory.area_rect(layer.id))
            })
            .expect("editor window should have an area");
        let client_rect = Rect::from_min_max(
            Pos2::new(window_rect.left() + 8.0, window_rect.top() + 30.0),
            Pos2::new(window_rect.right() - 8.0, window_rect.bottom() - 8.0),
        );
        let save_center = save_rect.center();
        assert!(
            client_rect.contains(save_rect.min) && client_rect.contains(save_rect.max),
            "save button bounds {save_rect:?} should be inside the window client area {client_rect:?} (window {window_rect:?})"
        );

        editor_window_frame(
            &ctx,
            &mut editor,
            size,
            vec![
                egui::Event::PointerMoved(save_center),
                egui::Event::PointerButton {
                    pos: save_center,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let (_, result) = editor_window_frame(
            &ctx,
            &mut editor,
            size,
            vec![egui::Event::PointerButton {
                pos: save_center,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(result, Some(editor.schema.clone()));
        crate::localization::set_language("RU");
    }
    #[test]
    fn color_parser_supports_rgb_and_rgba_and_rejects_bad_values() {
        assert_eq!(parse_color("#112233"), Some(Color32::from_rgb(17, 34, 51)));
        assert_eq!(
            parse_color("#11223344"),
            Some(Color32::from_rgba_unmultiplied(17, 34, 51, 68))
        );
        assert_eq!(parse_color("red"), None);
        assert_eq!(parse_color("#１２３４５６"), None);
    }

    #[test]
    fn label_modes_preserve_auto_blank_and_custom_draft() {
        assert_eq!(label_state(None).value(), None);
        assert_eq!(
            label_state(Some(&String::new())).value(),
            Some(String::new())
        );
        let mut custom = label_state(Some(&String::new()));
        custom.mode = LabelMode::Custom;
        custom.draft = "й".into();
        assert_eq!(custom.value(), Some("й".into()));
        custom.draft.clear();
        assert_eq!(custom.value(), Some(String::new()));
    }

    #[test]
    fn property_panel_renders_rect_and_polygon_fields_without_changing_schema() {
        let mut schema = blank_schema("Board", 16.0, 5.0);
        schema.keys.push(test_key(
            "KeyA",
            Geometry::Rect {
                x: 2.0,
                y: 1.0,
                width: 1.0,
                height: 1.0,
            },
        ));
        let mut editor = KeyboardEditor::new(schema);
        editor.select(0);
        let before = editor.schema.clone();
        editor.label_states[0].mode = LabelMode::Custom;
        editor.label_states[0].draft = "typed label".into();
        editor.label_states[1].mode = LabelMode::Blank;
        properties_frame(&Context::default(), &mut editor, 0);
        assert_eq!(editor.schema.keys[0].label.as_deref(), Some("typed label"));
        assert_eq!(editor.schema.keys[0].shifted_label.as_deref(), Some(""));
        editor.label_states[0].mode = LabelMode::Automatic;
        properties_frame(&Context::default(), &mut editor, 0);
        assert_eq!(editor.schema.keys[0].label, None);
        editor.schema.keys[0].geometry =
            toggle_geometry(&before.keys[0].geometry, before.canvas, true);
        editor.sync_polygon_text();
        properties_frame(&Context::default(), &mut editor, 0);
        editor.schema.validate().unwrap();
    }

    #[test]
    fn accessible_property_action_toggles_rectangle_to_polygon() {
        let mut schema = blank_schema("Board", 16.0, 5.0);
        schema.keys.push(test_key(
            "KeyA",
            Geometry::Rect {
                x: 2.0,
                y: 1.0,
                width: 1.0,
                height: 1.0,
            },
        ));
        let mut editor = KeyboardEditor::new(schema);
        let ctx = Context::default();
        ctx.enable_accesskit();
        let nodes = accessible_properties_frame(&ctx, &mut editor, 0, vec![]);
        let checkbox = nodes
            .iter()
            .find(|(_, node)| {
                node.role() == egui::accesskit::Role::CheckBox
                    && node.label() == Some("Многоугольник")
            })
            .expect("polygon toggle should be accessible")
            .0;
        accessible_properties_frame(&ctx, &mut editor, 0, vec![accesskit_click(checkbox)]);
        assert!(matches!(
            editor.schema.keys[0].geometry,
            Geometry::Polygon { .. }
        ));
    }

    #[test]
    fn accessible_delete_removes_selected_key_and_disables_delete_when_selection_is_empty() {
        crate::localization::set_language("EN");
        let mut schema = blank_schema("Board", 16.0, 5.0);
        for (id, x) in [("KeyA", 0.0), ("KeyB", 1.0)] {
            schema.keys.push(test_key(
                id,
                Geometry::Rect {
                    x,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                },
            ));
        }
        let mut editor = KeyboardEditor::new(schema);
        editor.select(1);
        let ctx = Context::default();
        ctx.enable_accesskit();
        let size = Vec2::new(900.0, 600.0);
        let mut delete_button = None;
        for _ in 0..3 {
            let (output, _) = editor_window_frame(&ctx, &mut editor, size, vec![]);
            delete_button = output
                .platform_output
                .accesskit_update
                .as_ref()
                .and_then(|update| {
                    update.nodes.iter().find_map(|(id, node)| {
                        (node.role() == egui::accesskit::Role::Button
                            && node.label() == Some("Delete selected"))
                        .then_some(*id)
                    })
                })
                .or(delete_button);
        }
        let delete_button = delete_button.expect("delete button should be accessible");
        editor_window_frame(
            &ctx,
            &mut editor,
            size,
            vec![accesskit_click(delete_button)],
        );

        assert_eq!(editor.selected, None);
        assert_eq!(editor.schema.keys.len(), 1);
        assert_eq!(editor.schema.keys[0].physical_id, "KeyA");

        let mut delete_disabled = false;
        for _ in 0..3 {
            let (output, _) = editor_window_frame(&ctx, &mut editor, size, vec![]);
            if let Some(update) = &output.platform_output.accesskit_update {
                delete_disabled |= update.nodes.iter().any(|(_, node)| {
                    node.role() == egui::accesskit::Role::Button
                        && node.label() == Some("Delete selected")
                        && node.is_disabled()
                });
            }
        }
        assert!(
            delete_disabled,
            "delete should be disabled without a selection"
        );
        assert_eq!(editor.schema.keys.len(), 1);
        assert_eq!(editor.schema.keys[0].physical_id, "KeyA");
        crate::localization::set_language("RU");
    }

    #[test]
    fn invalid_save_shows_error_keeps_draft_and_corrected_save_returns_copy() {
        crate::localization::set_language("EN");
        let mut schema = blank_schema("Draft board", 16.0, 5.0);
        schema.keys.push(test_key(
            "KeyA",
            Geometry::Rect {
                x: 15.5,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
        ));
        let mut editor = KeyboardEditor::new(schema.clone());
        editor.select(0);
        let ctx = Context::default();
        ctx.enable_accesskit();
        let size = Vec2::new(1200.0, 900.0);
        let mut save_button = None;
        for _ in 0..3 {
            let (output, _) = editor_window_frame(&ctx, &mut editor, size, vec![]);
            save_button = output
                .platform_output
                .accesskit_update
                .as_ref()
                .and_then(|update| {
                    update.nodes.iter().find_map(|(id, node)| {
                        (node.role() == egui::accesskit::Role::Button
                            && node.label() == Some("Validate & save copy"))
                        .then_some(*id)
                    })
                })
                .or(save_button);
        }
        let save_button = save_button.expect("save button should be accessible");
        let (_, result) =
            editor_window_frame(&ctx, &mut editor, size, vec![accesskit_click(save_button)]);
        assert!(result.is_none(), "invalid schema must not be returned");
        assert_eq!(editor.schema, schema, "validation must preserve the draft");
        let error = editor
            .error
            .as_deref()
            .expect("failed validation should set an error")
            .to_owned();
        assert!(error.contains("outside canvas"), "{error}");
        let mut error_is_accessible = false;
        for _ in 0..3 {
            let (output, _) = editor_window_frame(&ctx, &mut editor, size, vec![]);
            error_is_accessible |= output
                .platform_output
                .accesskit_update
                .as_ref()
                .is_some_and(|update| {
                    update.nodes.iter().any(|(_, node)| {
                        node.label() == Some(error.as_str()) || node.value() == Some(error.as_str())
                    })
                });
        }
        assert!(
            error_is_accessible,
            "validation error should be shown accessibly"
        );

        let Geometry::Rect { x, .. } = &mut editor.schema.keys[0].geometry else {
            unreachable!()
        };
        *x = 15.0;
        let mut save_button = None;
        for _ in 0..3 {
            let (output, _) = editor_window_frame(&ctx, &mut editor, size, vec![]);
            save_button = output
                .platform_output
                .accesskit_update
                .as_ref()
                .and_then(|update| {
                    update.nodes.iter().find_map(|(id, node)| {
                        (node.role() == egui::accesskit::Role::Button
                            && node.label() == Some("Validate & save copy"))
                        .then_some(*id)
                    })
                })
                .or(save_button);
        }
        let save_button = save_button.expect("save button should remain accessible");
        let expected = editor.schema.clone();
        let (_, result) =
            editor_window_frame(&ctx, &mut editor, size, vec![accesskit_click(save_button)]);
        assert_eq!(result, Some(expected));
        assert!(editor.error.is_none());
        crate::localization::set_language("RU");
    }

    #[test]
    fn polygon_editor_keeps_last_valid_shape_for_bad_points_then_restores_and_saves() {
        crate::localization::set_language("EN");
        let original_points = vec![
            Point { x: 2.0, y: 1.0 },
            Point { x: 4.0, y: 1.0 },
            Point { x: 4.0, y: 2.0 },
            Point { x: 2.0, y: 2.0 },
        ];
        let mut schema = blank_schema("Polygon draft", 16.0, 5.0);
        schema.keys.push(test_key(
            "Enter",
            Geometry::Polygon {
                points: original_points.clone(),
            },
        ));
        let mut editor = KeyboardEditor::new(schema);
        editor.select(0);
        let original_text = editor.polygon_text.clone();
        let ctx = Context::default();
        ctx.enable_accesskit();
        let size = Vec2::new(1200.0, 900.0);
        let mut polygon_editor = None;
        for _ in 0..3 {
            let (output, _) = editor_window_frame(&ctx, &mut editor, size, vec![]);
            polygon_editor = output
                .platform_output
                .accesskit_update
                .as_ref()
                .and_then(|update| {
                    update.nodes.iter().find_map(|(id, node)| {
                        (node.role() == egui::accesskit::Role::MultilineTextInput
                            && node.value() == Some(original_text.as_str()))
                        .then_some(*id)
                    })
                })
                .or(polygon_editor);
        }
        let polygon_editor = polygon_editor.expect("polygon points editor should be accessible");

        let invalid_draft = "2,1\nnot-a-point\n4,2\n2,2";
        editor_window_frame(
            &ctx,
            &mut editor,
            size,
            vec![accesskit_set_value(polygon_editor, invalid_draft)],
        );
        assert_eq!(editor.polygon_text, invalid_draft);
        assert_eq!(
            editor.schema.keys[0].geometry,
            Geometry::Polygon {
                points: original_points.clone()
            },
            "invalid point drafts must preserve the last valid polygon"
        );

        let restored_points = vec![
            Point { x: 3.0, y: 1.0 },
            Point { x: 5.0, y: 1.0 },
            Point { x: 5.0, y: 2.0 },
            Point { x: 3.0, y: 2.0 },
        ];
        let restored_text = "3,1\n5,1\n5,2\n3,2";
        editor_window_frame(
            &ctx,
            &mut editor,
            size,
            vec![accesskit_set_value(polygon_editor, restored_text)],
        );
        assert_eq!(editor.polygon_text, restored_text);
        assert_eq!(
            editor.schema.keys[0].geometry,
            Geometry::Polygon {
                points: restored_points
            }
        );

        let mut save_button = None;
        for _ in 0..3 {
            let (output, _) = editor_window_frame(&ctx, &mut editor, size, vec![]);
            save_button = output
                .platform_output
                .accesskit_update
                .as_ref()
                .and_then(|update| {
                    update.nodes.iter().find_map(|(id, node)| {
                        (node.role() == egui::accesskit::Role::Button
                            && node.label() == Some("Validate & save copy"))
                        .then_some(*id)
                    })
                })
                .or(save_button);
        }
        let save_button = save_button.expect("save button should be accessible");
        let expected = editor.schema.clone();
        let (_, result) =
            editor_window_frame(&ctx, &mut editor, size, vec![accesskit_click(save_button)]);
        assert_eq!(result, Some(expected));
        crate::localization::set_language("RU");
    }

    #[test]
    fn accessible_finger_combobox_assignment_is_preserved_in_saved_copy() {
        crate::localization::set_language("EN");
        let mut schema = blank_schema("Finger draft", 16.0, 5.0);
        schema.keys.push(test_key(
            "KeyA",
            Geometry::Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
        ));
        let mut editor = KeyboardEditor::new(schema);
        editor.select(0);
        let ctx = Context::default();
        ctx.enable_accesskit();
        let size = Vec2::new(1200.0, 900.0);

        let mut finger_combo = None;
        for _ in 0..3 {
            let (output, _) = editor_window_frame(&ctx, &mut editor, size, vec![]);
            finger_combo = output
                .platform_output
                .accesskit_update
                .as_ref()
                .and_then(|update| {
                    update.nodes.iter().find_map(|(id, node)| {
                        (node.role() == egui::accesskit::Role::ComboBox
                            && node.value() == Some("None"))
                        .then_some(*id)
                    })
                })
                .or(finger_combo);
        }
        let finger_combo = finger_combo.expect("finger selector should be accessible");
        editor_window_frame(&ctx, &mut editor, size, vec![accesskit_click(finger_combo)]);

        let mut left_index_option = None;
        for _ in 0..3 {
            let (output, _) = editor_window_frame(&ctx, &mut editor, size, vec![]);
            left_index_option = output
                .platform_output
                .accesskit_update
                .as_ref()
                .and_then(|update| {
                    update.nodes.iter().find_map(|(id, node)| {
                        (node.role() == egui::accesskit::Role::Button
                            && node.label() == Some("Left index"))
                        .then_some(*id)
                    })
                })
                .or(left_index_option);
        }
        let left_index_option =
            left_index_option.expect("left index option should be accessible in the popup");
        editor_window_frame(
            &ctx,
            &mut editor,
            size,
            vec![accesskit_click(left_index_option)],
        );
        assert_eq!(editor.schema.keys[0].finger, Some(Finger::LeftIndex));

        let mut save_button = None;
        for _ in 0..3 {
            let (output, _) = editor_window_frame(&ctx, &mut editor, size, vec![]);
            save_button = output
                .platform_output
                .accesskit_update
                .as_ref()
                .and_then(|update| {
                    update.nodes.iter().find_map(|(id, node)| {
                        (node.role() == egui::accesskit::Role::Button
                            && node.label() == Some("Validate & save copy"))
                        .then_some(*id)
                    })
                })
                .or(save_button);
        }
        let save_button = save_button.expect("save button should be accessible");
        let expected = editor.schema.clone();
        let (_, result) =
            editor_window_frame(&ctx, &mut editor, size, vec![accesskit_click(save_button)]);
        let saved = result.expect("valid key finger assignment should save");
        assert_eq!(saved.keys[0].finger, Some(Finger::LeftIndex));
        assert_eq!(saved, expected);
        crate::localization::set_language("RU");
    }

    fn accesskit_click(target_node: egui::accesskit::NodeId) -> egui::Event {
        egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node,
            data: None,
        })
    }

    fn accesskit_set_value(target_node: egui::accesskit::NodeId, value: &str) -> egui::Event {
        egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::SetValue,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node,
            data: Some(egui::accesskit::ActionData::Value(value.into())),
        })
    }

    #[test]
    fn shape_toggle_preserves_small_canvas_bounds_and_polygon_parser_rejects_bad_vertices() {
        let canvas = Canvas {
            width: 16.0,
            height: 5.0,
        };
        let rect = Geometry::Rect {
            x: 3.0,
            y: 1.0,
            width: 1.0,
            height: 1.0,
        };
        let polygon = toggle_geometry(&rect, canvas, true);
        let Geometry::Polygon { points } = &polygon else {
            unreachable!()
        };
        assert_eq!(points.len(), 4);
        let round_trip = toggle_geometry(&polygon, canvas, false);
        assert_eq!(round_trip, rect);
        let valid_text = "3,1\n4,1\n4,2\n3,2";
        assert_eq!(parse_polygon_points(valid_text).unwrap(), points.clone());
        assert!(parse_polygon_points("3,1\nnot-a-point\n4,2").is_err());
        let mut edited = points.clone();
        assert!(apply_polygon_text(&mut edited, "2,1\n4,1\n4,2\n2,2"));
        assert_eq!(edited[0].x, 2.0);
        let before_invalid_edit = edited.clone();
        assert!(!apply_polygon_text(&mut edited, "invalid"));
        assert!(!apply_polygon_text(&mut edited, ""));
        assert_eq!(
            edited, before_invalid_edit,
            "bad drafts must not destroy the last valid geometry"
        );
        assert!(matches!(
            toggle_geometry(&Geometry::Polygon { points: vec![] }, canvas, false),
            Geometry::Rect { .. }
        ));
        let mut schema = blank_schema("Board", 16.0, 5.0);
        schema.keys.push(test_key("KeyA", polygon));
        schema.validate().unwrap();
    }

    #[test]
    fn add_select_delete_and_validated_copy_preserve_schema_contract() {
        let mut editor = KeyboardEditor::new(blank_schema("Board", 16.0, 5.0));
        editor.add_key();
        assert_eq!(editor.schema.keys.len(), 1);
        assert_eq!(editor.selected, Some(0));
        editor.schema.keys[0].physical_id = "KeyA".into();
        editor.select(0);
        assert_eq!(editor.label_states[0].value(), None);
        let copied = editor
            .validated_copy()
            .expect("valid schema should be returned");
        assert_eq!(copied, editor.schema);
        copied.validate().unwrap();
        editor.delete_selected();
        assert!(editor.schema.keys.is_empty());
    }

    #[test]
    fn malformed_geometry_stays_in_editor_and_reports_validation_error() {
        let mut schema = blank_schema("Board", 16.0, 5.0);
        schema.keys.push(test_key(
            "KeyA",
            Geometry::Rect {
                x: 15.5,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
        ));
        let mut editor = KeyboardEditor::new(schema);
        editor.select(0);
        assert!(editor.validated_copy().is_none());
        assert!(
            editor
                .error
                .as_deref()
                .is_some_and(|message| message.contains("outside canvas"))
        );
        assert_eq!(editor.schema.keys.len(), 1);
    }

    #[test]
    fn canvas_drag_moves_and_resizes_rectangle_and_edits_polygon_vertex() {
        let mut schema = blank_schema("Board", 16.0, 5.0);
        schema.keys.push(test_key(
            "KeyA",
            Geometry::Rect {
                x: 2.0,
                y: 1.0,
                width: 1.0,
                height: 1.0,
            },
        ));
        schema.keys.push(test_key(
            "Enter",
            Geometry::Polygon {
                points: vec![
                    Point { x: 4.0, y: 1.0 },
                    Point { x: 5.0, y: 1.0 },
                    Point { x: 5.0, y: 2.0 },
                    Point { x: 4.0, y: 2.0 },
                ],
            },
        ));
        let mut editor = KeyboardEditor::new(schema);
        let ctx = Context::default();
        let output = canvas_frame(&ctx, &mut editor, vec![]);
        let board = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) if rect.fill == Color32::from_gray(32) => Some(rect.rect),
                _ => None,
            })
            .expect("canvas should be painted");
        let scale = board.width() / 16.0;
        let center = board.min + Vec2::new(2.5 * scale, 1.5 * scale);
        drag(&ctx, &mut editor, center, center + Vec2::new(scale, 0.0));
        let Geometry::Rect { x, .. } = editor.schema.keys[0].geometry else {
            unreachable!()
        };
        assert!((x - 3.0).abs() < 0.1);
        editor.schema.keys.pop();

        let output = canvas_frame(&ctx, &mut editor, vec![]);
        let board = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) if rect.fill == Color32::from_gray(32) => Some(rect.rect),
                _ => None,
            })
            .unwrap();
        let scale = board.width() / 16.0;
        let handle = board.min + Vec2::new(3.95 * scale, 1.95 * scale);
        drag(&ctx, &mut editor, handle, handle + Vec2::new(scale, scale));
        let Geometry::Rect { width, height, .. } = editor.schema.keys[0].geometry else {
            unreachable!()
        };
        assert!(
            width > 1.5 && height > 1.5,
            "resized rectangle: {width} × {height}"
        );
        editor.schema.keys.push(test_key(
            "Enter",
            Geometry::Polygon {
                points: vec![
                    Point { x: 4.0, y: 1.0 },
                    Point { x: 5.0, y: 1.0 },
                    Point { x: 5.0, y: 2.0 },
                    Point { x: 4.0, y: 2.0 },
                ],
            },
        ));

        let output = canvas_frame(&ctx, &mut editor, vec![]);
        let board = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) if rect.fill == Color32::from_gray(32) => Some(rect.rect),
                _ => None,
            })
            .unwrap();
        let scale = board.width() / 16.0;
        let vertex = board.min + Vec2::new(4.0 * scale, 1.0 * scale);
        drag(
            &ctx,
            &mut editor,
            vertex,
            vertex + Vec2::new(scale * 0.5, 0.0),
        );
        let Geometry::Polygon { points } = &editor.schema.keys[1].geometry else {
            unreachable!()
        };
        assert!((points[0].x - 4.5).abs() < 0.15);
    }

    #[test]
    fn rectangle_resize_is_clamped_to_canvas_edges_across_drag_frames() {
        let mut schema = blank_schema("Edge board", 16.0, 5.0);
        schema.keys.push(test_key(
            "KeyA",
            Geometry::Rect {
                x: 14.5,
                y: 3.5,
                width: 1.0,
                height: 1.0,
            },
        ));
        let mut editor = KeyboardEditor::new(schema);
        let ctx = Context::default();
        let output = canvas_frame(&ctx, &mut editor, vec![]);
        let board = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == Color32::from_gray(32) => Some(r.rect),
                _ => None,
            })
            .unwrap();
        let scale = board.width() / 16.0;
        let near_corner = board.min + Vec2::new(15.45 * scale, 4.45 * scale);
        drag(
            &ctx,
            &mut editor,
            near_corner,
            near_corner + Vec2::new(3.0 * scale, 3.0 * scale),
        );
        let Geometry::Rect {
            x,
            y,
            width,
            height,
        } = editor.schema.keys[0].geometry
        else {
            unreachable!()
        };
        assert!((x + width - 16.0).abs() < 0.02);
        assert!((y + height - 5.0).abs() < 0.02);
        editor.schema.validate().unwrap();
    }

    #[test]
    fn polygon_boundary_translation_preserves_shape_instead_of_clamping_vertices_individually() {
        let mut schema = blank_schema("Edge board", 16.0, 5.0);
        schema.keys.push(test_key(
            "Enter",
            Geometry::Polygon {
                points: vec![
                    Point { x: 14.0, y: 3.0 },
                    Point { x: 15.0, y: 3.0 },
                    Point { x: 15.0, y: 4.0 },
                    Point { x: 14.0, y: 4.0 },
                ],
            },
        ));
        let mut editor = KeyboardEditor::new(schema);
        let ctx = Context::default();
        let output = canvas_frame(&ctx, &mut editor, vec![]);
        let board = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == Color32::from_gray(32) => Some(r.rect),
                _ => None,
            })
            .unwrap();
        let scale = board.width() / 16.0;
        let center = board.min + Vec2::new(14.5 * scale, 3.5 * scale);
        drag(
            &ctx,
            &mut editor,
            center,
            center + Vec2::new(3.0 * scale, 3.0 * scale),
        );
        let Geometry::Polygon { points } = &editor.schema.keys[0].geometry else {
            unreachable!()
        };
        let xs: Vec<f64> = points.iter().map(|p| p.x).collect();
        let ys: Vec<f64> = points.iter().map(|p| p.y).collect();
        assert!((xs.iter().copied().fold(f64::INFINITY, f64::min) - 15.0).abs() < 0.02);
        assert!((xs.iter().copied().fold(f64::NEG_INFINITY, f64::max) - 16.0).abs() < 0.02);
        assert!((ys.iter().copied().fold(f64::INFINITY, f64::min) - 4.0).abs() < 0.02);
        assert!((ys.iter().copied().fold(f64::NEG_INFINITY, f64::max) - 5.0).abs() < 0.02);
        assert_eq!(points[0].x, points[3].x);
        assert_eq!(points[0].y, points[1].y);
        editor.schema.validate().unwrap();
    }

    #[test]
    fn concave_polygon_triangulation_preserves_notch_and_editor_draws_outline() {
        let points = vec![
            Point { x: 1.0, y: 1.0 },
            Point { x: 4.0, y: 1.0 },
            Point { x: 4.0, y: 2.0 },
            Point { x: 2.0, y: 2.0 },
            Point { x: 2.0, y: 4.0 },
            Point { x: 1.0, y: 4.0 },
        ];
        let screen: Vec<Pos2> = points
            .iter()
            .map(|p| Pos2::new(p.x as f32, p.y as f32))
            .collect();
        let triangles = crate::keyboard::triangulate(&screen);
        assert_eq!(triangles.len(), points.len() - 2);
        let triangle_area: f32 = triangles
            .iter()
            .map(|t| {
                ((t[1].x - t[0].x) * (t[2].y - t[0].y) - (t[1].y - t[0].y) * (t[2].x - t[0].x))
                    .abs()
                    * 0.5
            })
            .sum();
        assert!(
            (triangle_area - 5.0).abs() < 0.001,
            "triangulation must not fill the concave notch"
        );

        let mut schema = blank_schema("Concave", 16.0, 5.0);
        schema
            .keys
            .push(test_key("KeyA", Geometry::Polygon { points }));
        let mut editor = KeyboardEditor::new(schema);
        let output = canvas_frame(&Context::default(), &mut editor, vec![]);
        assert!(output.shapes.iter().any(
            |s| matches!(&s.shape, egui::Shape::Path(path) if path.closed && path.points.len() == 6)
        ));
    }

    #[test]
    fn polygon_selection_and_interior_drag_keep_key_properties_and_move_all_points() {
        let mut schema = blank_schema("Board", 16.0, 5.0);
        schema.keys.push(test_key(
            "Enter",
            Geometry::Polygon {
                points: vec![
                    Point { x: 4.0, y: 1.0 },
                    Point { x: 5.0, y: 1.0 },
                    Point { x: 5.0, y: 2.0 },
                    Point { x: 4.0, y: 2.0 },
                ],
            },
        ));
        let mut editor = KeyboardEditor::new(schema);
        let ctx = Context::default();
        let output = canvas_frame(&ctx, &mut editor, vec![]);
        let board = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(rect) if rect.fill == Color32::from_gray(32) => Some(rect.rect),
                _ => None,
            })
            .unwrap();
        let scale = board.width() / 16.0;
        let center = board.min + Vec2::new(4.5 * scale, 1.5 * scale);
        click_canvas(&ctx, &mut editor, center);
        assert_eq!(editor.selected, Some(0));
        assert!(editor.polygon_text.contains("4,1"));
        drag(&ctx, &mut editor, center, center + Vec2::new(scale, 0.0));
        let Geometry::Polygon { points } = &editor.schema.keys[0].geometry else {
            unreachable!()
        };
        let mut xs: Vec<f64> = points.iter().map(|p| p.x).collect();
        xs.sort_by(f64::total_cmp);
        assert!(
            xs.iter().all(|x| *x >= 4.9 && *x <= 6.1),
            "polygon moved to {xs:?}"
        );
        assert!((xs[0] - 5.0).abs() < 0.15 && (xs[3] - 6.0).abs() < 0.15);
        assert!(editor.polygon_text.contains("6,1"));
    }

    #[test]
    fn dragging_unselected_key_loads_its_own_labels_before_properties_write_back() {
        let mut schema = blank_schema("Board", 16.0, 5.0);
        schema.keys.push(test_key(
            "KeyA",
            Geometry::Rect {
                x: 1.0,
                y: 1.0,
                width: 1.0,
                height: 1.0,
            },
        ));
        schema.keys[0].label = Some("first".into());
        schema.keys[0].shifted_label = Some("FIRST".into());
        schema.keys.push(test_key(
            "Enter",
            Geometry::Polygon {
                points: vec![
                    Point { x: 5.0, y: 1.0 },
                    Point { x: 6.0, y: 1.0 },
                    Point { x: 6.0, y: 2.0 },
                    Point { x: 5.0, y: 2.0 },
                ],
            },
        ));
        schema.keys[1].label = Some("second".into());
        schema.keys[1].shifted_label = Some("SECOND".into());
        let mut editor = KeyboardEditor::new(schema);
        editor.select(0);
        let ctx = Context::default();
        let output = canvas_frame(&ctx, &mut editor, vec![]);
        let board = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(rect) if rect.fill == Color32::from_gray(32) => Some(rect.rect),
                _ => None,
            })
            .unwrap();
        let scale = board.width() / 16.0;
        let center = board.min + Vec2::new(5.5 * scale, 1.5 * scale);
        drag(
            &ctx,
            &mut editor,
            center,
            center + Vec2::new(scale * 0.5, 0.0),
        );

        assert_eq!(editor.selected, Some(1));
        assert_eq!(editor.label_states[0].value(), Some("second".into()));
        assert_eq!(editor.label_states[1].value(), Some("SECOND".into()));
        properties_frame(&ctx, &mut editor, 1);
        assert_eq!(editor.schema.keys[1].label.as_deref(), Some("second"));
        assert_eq!(
            editor.schema.keys[1].shifted_label.as_deref(),
            Some("SECOND")
        );
        assert!(editor.polygon_text.contains("5.5,1"));
    }

    #[test]
    fn key_limit_and_empty_delete_are_safe() {
        let mut editor = KeyboardEditor::new(blank_schema("Full", 16.0, 5.0));
        editor.delete_selected();
        assert!(editor.schema.keys.is_empty());
        for index in 0..256 {
            editor.schema.keys.push(test_key(
                &format!("Key{}", (b'A' + (index % 26) as u8) as char),
                Geometry::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.1,
                    height: 0.1,
                },
            ));
        }
        editor.add_key();
        assert_eq!(editor.schema.keys.len(), 256);
        assert!(editor.error.as_deref().is_some_and(|e| e.contains("256")));
    }

    fn drag(ctx: &Context, editor: &mut KeyboardEditor, from: Pos2, to: Pos2) {
        canvas_frame(
            ctx,
            editor,
            vec![
                egui::Event::PointerMoved(from),
                egui::Event::PointerButton {
                    pos: from,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let midpoint = from + (to - from) * 0.5;
        canvas_frame(ctx, editor, vec![egui::Event::PointerMoved(midpoint)]);
        canvas_frame(ctx, editor, vec![egui::Event::PointerMoved(to)]);
        canvas_frame(
            ctx,
            editor,
            vec![egui::Event::PointerButton {
                pos: to,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }

    fn click_canvas(ctx: &Context, editor: &mut KeyboardEditor, pos: Pos2) {
        canvas_frame(
            ctx,
            editor,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        canvas_frame(
            ctx,
            editor,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
}
