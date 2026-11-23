use super::*;
use eframe::egui::{Pos2, Rect, Response};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewportArea {
    Scene,
    Physics,
    Shading,
}

pub struct LiveDrag {
    transform: Transform,
    movement: Vec2,
}

#[derive(Clone, Copy)]
enum Command {
    Add(mesh::Primitive),
    Object(Kind),
    Role(scene::FluidRole),
    Transform(Operation),
    Edit,
    Frame,
    Physics,
    LiveMove,
    LiveRotate,
    Split,
}

impl App {
    pub(super) fn area_actions(&mut self, ui: &mut egui::Ui) {
        if ui
            .button(if self.split_view {
                "Join Area"
            } else {
                "Split Area"
            })
            .clicked()
        {
            self.area = ViewportArea::Scene;
            self.split_view = !self.split_view;
            self.dirty = true;
            ui.close();
        }
        if ui.button("Maximize Area    Ctrl+Space").clicked() {
            self.maximized = !self.maximized;
            self.dirty = true;
            ui.close();
        }
    }

    pub(super) fn viewport_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("viewport editor type")
                .selected_text(match self.area {
                    ViewportArea::Scene => "3D",
                    ViewportArea::Physics => "Physics",
                    ViewportArea::Shading => "Shading",
                })
                .width(45.0)
                .show_ui(ui, |ui| {
                    for (area, label) in [
                        (ViewportArea::Scene, "3D Viewport"),
                        (ViewportArea::Physics, "Physics"),
                        (ViewportArea::Shading, "Shading"),
                    ] {
                        if ui.selectable_value(&mut self.area, area, label).changed() {
                            self.dirty = true;
                        }
                    }
                });
            if self.area != ViewportArea::Scene {
                return;
            }
            ui.menu_button(
                if self.edit.is_some() {
                    "Edit Mode"
                } else {
                    "Object Mode"
                },
                |ui| {
                    if ui.button("Toggle Edit Mode    Tab").clicked() {
                        self.toggle_edit();
                        ui.close();
                    }
                },
            );
            ui.menu_button("View", |ui| {
                if ui.button("Frame All    Home").clicked() {
                    self.frame_scene();
                    ui.close();
                }
                if ui
                    .checkbox(&mut self.camera.ortho, "Orthographic")
                    .changed()
                {
                    self.dirty = true;
                }
                if ui.button("Viewport Camera").clicked() {
                    self.properties = Properties::Object;
                    self.camera_properties = true;
                    ui.close();
                }
                ui.separator();
                self.area_actions(ui);
            });
            ui.menu_button("Select", |ui| {
                if ui.button("Deselect").clicked() {
                    if self.edit.is_some() {
                        self.toggle_edit();
                    }
                    self.selected = None;
                    self.dirty = true;
                    ui.close();
                }
                if ui.button("Search Operators    F3").clicked() {
                    self.open_search();
                    ui.close();
                }
            });
            if ui.button("Add").on_hover_text("Shift+A").clicked() {
                self.add = true;
            }
            ui.menu_button(
                if self.edit.is_some() {
                    "Mesh"
                } else {
                    "Object"
                },
                |ui| {
                    if ui.button("Adjust Last Operation    F9").clicked() {
                        self.adjust_last = self.redo_transform.is_some();
                        ui.close();
                    }
                    if self.edit.is_none() && ui.button("Duplicate    Shift+D").clicked() {
                        self.duplicate();
                        ui.close();
                    }
                },
            );
            ui.separator();
            ui.checkbox(&mut self.snapping, "Snap");
            ui.menu_button("Live", |ui| {
                self.live_tools(ui);
                ui.separator();
                self.section_controls(ui);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (mode, label) in [
                    (Display::RayTraced, "●"),
                    (Display::Surface, "S"),
                    (Display::Solid, "○"),
                    (Display::Particles, "P"),
                ] {
                    self.dirty |= ui
                        .selectable_value(&mut self.mode, mode, label)
                        .on_hover_text(mode.label())
                        .changed();
                }
                if ui.button("⚙").on_hover_text("Viewport shading").clicked() {
                    self.shading = !self.shading;
                }
            });
        });
    }

    pub(super) fn section_controls(&mut self, ui: &mut egui::Ui) {
        let mut enabled = self.cross_section.is_some();
        if ui.checkbox(&mut enabled, "Cross Section").changed() {
            self.cross_section = enabled.then_some(1);
            if enabled {
                let (_, _, _, forward) = self.camera.basis();
                self.cross_section = Some(if forward.x.abs() > forward.z.abs() {
                    0
                } else {
                    1
                });
                self.section_reversed = self
                    .cross_section
                    .is_some_and(|axis| input::AXES[axis].dot(forward) > 0.0);
                self.section_offset = 0.0;
            }
            self.dirty = true;
        }
        if let Some(axis) = &mut self.cross_section {
            ui.horizontal(|ui| {
                for (i, name) in ["X", "Y", "Z"].into_iter().enumerate() {
                    self.dirty |= ui.selectable_value(axis, i, name).changed();
                }
            });
            self.dirty |= ui
                .checkbox(&mut self.section_reversed, "Flip Side")
                .changed();
            self.dirty |= slider(
                ui,
                "Slice Position",
                &mut self.section_offset,
                -2.0..=2.8,
                2,
            );
        }
    }

    pub(super) fn live_tools(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .selectable_value(&mut self.live_tool, None, "Select")
                .clicked()
            {
                ui.ctx().memory_mut(|m| m.stop_text_input());
            }
            if ui
                .selectable_value(&mut self.live_tool, Some(Operation::Move), "Grab")
                .on_hover_text("Drag a collider in the viewport")
                .clicked()
            {
                ui.ctx().memory_mut(|m| m.stop_text_input());
            }
            if ui
                .selectable_value(&mut self.live_tool, Some(Operation::Rotate), "Tip")
                .on_hover_text("Drag to rotate a collider")
                .clicked()
            {
                ui.ctx().memory_mut(|m| m.stop_text_input());
            }
        });
    }

    pub(super) fn live_pointer(
        &mut self,
        ctx: &egui::Context,
        rect: Rect,
        response: &Response,
    ) -> bool {
        if self.edit.is_some()
            || self.transform.is_some()
            || self.search
            || self.file
            || self.help
            || self.add
        {
            return false;
        }
        if let Some(point) = response.interact_pointer_pos()
            && response.clicked_by(PointerButton::Secondary)
            && ctx.input(|i| i.modifiers.shift)
        {
            let uv = (point - rect.min) / rect.size();
            let (origin, ray) = self
                .camera
                .ray(glam::Vec2::new(uv.x, uv.y), rect.width() / rect.height());
            if ray.y.abs() > 0.001 {
                let t = -origin.y / ray.y;
                if t > 0.0 {
                    self.cursor = (origin + ray * t).clamp(fluid::ORIGIN, fluid::END);
                    self.cursor_placed = true;
                    self.dirty = true;
                }
            }
            return true;
        }
        let Some(tool) = self.live_tool else {
            return false;
        };
        let mut started = false;
        if self.live_drag.is_none()
            && response.drag_started_by(PointerButton::Primary)
            && !ctx.input(|i| i.modifiers.alt)
            && let Some(point) = ctx.input(|i| i.pointer.press_origin())
        {
            let tick = (self.snapshot.time / DT).round() as u32;
            let scene = self.scene.at_frame(tick);
            if let Some(id) = input::pick(&scene, self.camera, rect, point) {
                let target = scene.objects.iter().find(|o| o.id == id);
                if let Some(target) = target
                    && (target.is_collider() || target.kind == Kind::Field)
                {
                    let mut transform = Transform::new(&self.scene, id, tool).unwrap();
                    let original = &transform.original;
                    self.live_origins.entry(id).or_insert((
                        original.position,
                        original.rotation,
                        original.motion,
                    ));
                    transform.original = target.clone();
                    transform.original.motion = None;
                    self.selected = Some(id);
                    self.show_frame(None);
                    self.playing = true;
                    self.adaptive_quality = self.render_quality;
                    let movement = response.drag_delta();
                    self.live_drag = Some(LiveDrag {
                        transform,
                        movement,
                    });
                    started = true;
                }
            }
        }
        let Some(drag) = &mut self.live_drag else {
            return false;
        };
        let finished = response.drag_stopped_by(PointerButton::Primary);
        let cancel = ctx.input(|i| i.key_pressed(Key::Escape));
        if cancel {
            self.scene = self.live_drag.take().unwrap().transform.before;
            self.dirty = true;
            return true;
        }
        if !started {
            drag.movement += ctx.input(|i| i.pointer.delta());
        }
        drag.transform.delta = drag.movement;
        let mut goal = drag.transform.original.clone();
        drag.transform.apply(&mut goal, self.camera, rect.height());
        if let Some(object) = self.scene.objects.iter_mut().find(|o| o.id == goal.id) {
            object.motion = None;
            let delta = goal.position - object.position;
            object.position += delta.clamp_length_max(0.085);
            let angle = object.rotation.angle_between(goal.rotation);
            object.rotation = object
                .rotation
                .slerp(goal.rotation, (0.17 / angle.max(0.001)).min(1.0));
            self.dirty = true;
        }
        if finished || !ctx.input(|i| i.pointer.primary_down()) {
            let before = self.live_drag.take().unwrap().transform.before;
            self.remember(before);
        }
        true
    }

    pub(super) fn alternate_editor(&mut self, ui: &mut egui::Ui) {
        let changed = egui::ScrollArea::vertical()
            .id_salt("alternate editor")
            .show(ui, |ui| match self.area {
                ViewportArea::Physics => self.fluid_properties(ui),
                ViewportArea::Shading => self.render_controls(ui),
                ViewportArea::Scene => None,
            })
            .inner;
        if changed.is_some() {
            self.dirty = true;
        }
        self.remember_inspector(ui.ctx(), changed);
    }

    pub(super) fn show_view_split(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let divider = Rect::from_min_max(
            egui::pos2(rect.right() - 2.0, rect.top()),
            egui::pos2(rect.right() + 4.0, rect.bottom()),
        );
        let resize = ui
            .interact(divider, ui.id().with("split divider"), egui::Sense::drag())
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        ui.painter().vline(
            rect.right() + 1.0,
            rect.y_range(),
            Stroke::new(2.0_f32, LINE),
        );
        if resize.dragged()
            && let Some(cursor) = resize.interact_pointer_pos()
        {
            self.split_fraction = ((cursor.x - rect.left())
                / (ui.max_rect().right() - rect.left()))
            .clamp(0.43, 0.81);
            self.dirty = true;
        }
        let bounds = Rect::from_min_max(
            egui::pos2(rect.right() + 2.0, rect.top()),
            ui.max_rect().max,
        );
        let mut area = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("viewport split")
                .max_rect(bounds),
        );
        area.set_clip_rect(bounds);
        let mut changed = None;
        area.vertical(|ui| {
            ui.set_width(bounds.width());
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("split editor")
                    .selected_text(if self.split_editor == ViewportArea::Shading {
                        "Shading"
                    } else {
                        "Physics"
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.split_editor,
                            ViewportArea::Physics,
                            "Physics",
                        );
                        ui.selectable_value(
                            &mut self.split_editor,
                            ViewportArea::Shading,
                            "Shading",
                        );
                    });
                if ui.small_button("×").on_hover_text("Join Area").clicked() {
                    self.split_view = false;
                    self.dirty = true;
                }
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("split scroll")
                .show(ui, |ui| {
                    changed = if self.split_editor == ViewportArea::Shading {
                        self.render_controls(ui)
                    } else {
                        self.fluid_properties(ui)
                    };
                });
        });
        self.remember_inspector(ui.ctx(), changed);
    }

    pub(super) fn layout_overlays(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        painter: &egui::Painter,
    ) {
        if let Some(point) = input::project(self.camera, rect, self.cursor) {
            painter.circle_stroke(point, 7.0, Stroke::new(1.0_f32, Color32::WHITE));
            for quadrant in [0.0_f32, std::f32::consts::PI] {
                let arc = (0..=6)
                    .map(|i| {
                        let angle = quadrant + i as f32 * std::f32::consts::PI / 12.0;
                        point + egui::vec2(angle.cos(), angle.sin()) * 7.0
                    })
                    .collect::<Vec<_>>();
                painter.add(egui::Shape::line(
                    arc,
                    Stroke::new(1.5_f32, Color32::from_rgb(229, 64, 64)),
                ));
            }
            for axis in [Vec2::X, Vec2::Y] {
                painter.line_segment(
                    [point - axis * 11.0, point + axis * 11.0],
                    Stroke::new(1.0_f32, Color32::from_gray(27)),
                );
            }
        }
        if self
            .scene
            .objects
            .iter()
            .any(|o| o.is_domain() && o.visible)
        {
            let (lo, hi) = self.scene.domain_bounds();
            let points: [Option<Pos2>; 8] = std::array::from_fn(|i| {
                input::project(
                    self.camera,
                    rect,
                    Vec3::new(
                        if i & 1 == 0 { lo.x } else { hi.x },
                        if i & 2 == 0 { lo.y } else { hi.y },
                        if i & 4 == 0 { lo.z } else { hi.z },
                    ),
                )
            });
            for i in 0..8 {
                for axis in [1, 2, 4] {
                    if i & axis == 0
                        && let (Some(a), Some(b)) = (points[i], points[i | axis])
                    {
                        painter.line_segment(
                            [a, b],
                            Stroke::new(
                                1.1_f32,
                                if self.selection().is_some_and(Object::is_domain) {
                                    ACCENT
                                } else {
                                    MUTED
                                },
                            ),
                        );
                    }
                }
            }
        }
        let center = egui::pos2(rect.right() - 48.0, rect.top() + 48.0);
        let (_, right, up, forward) = self.camera.basis();
        let mut axes = [
            (
                "X",
                Vec3::X,
                Color32::from_rgb(220, 81, 91),
                std::f32::consts::FRAC_PI_2,
                0.0,
            ),
            ("Y", Vec3::NEG_Z, Color32::from_rgb(142, 174, 55), 0.0, 0.0),
            (
                "Z",
                Vec3::Y,
                Color32::from_rgb(79, 146, 242),
                self.camera.yaw,
                1.569,
            ),
        ];
        axes.sort_by(|a, b| forward.dot(b.1).total_cmp(&forward.dot(a.1)));
        for (label, axis, color, yaw, pitch) in axes {
            let tip = center + egui::vec2(axis.dot(right), -axis.dot(up)) * 25.0;
            painter.line_segment([center, tip], Stroke::new(1.2_f32, color));
            painter.circle_filled(tip, 8.0, color);
            painter.text(
                tip,
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(11.0),
                Color32::BLACK,
            );
            if ui
                .interact(
                    Rect::from_center_size(tip, Vec2::splat(17.0)),
                    ui.id().with(label),
                    egui::Sense::click(),
                )
                .clicked()
            {
                self.camera.yaw = yaw;
                self.camera.pitch = pitch;
                self.camera.ortho = true;
                self.dirty = true;
            }
        }
    }

    pub(super) fn open_search(&mut self) {
        self.query.clear();
        self.search_cursor = 0;
        self.search = true;
    }

    fn commands(&self) -> Vec<(String, Command)> {
        let mut list = vec![
            ("Toggle Edit Mode".into(), Command::Edit),
            ("Frame All".into(), Command::Frame),
            ("Fluid Properties".into(), Command::Physics),
            ("Grab Live".into(), Command::LiveMove),
            ("Tip Live".into(), Command::LiveRotate),
            ("Split or Join Area".into(), Command::Split),
        ];
        if self.selection().is_some() && self.edit.is_none() {
            list.extend([
                ("Move".into(), Command::Transform(Operation::Move)),
                ("Rotate".into(), Command::Transform(Operation::Rotate)),
                ("Scale".into(), Command::Transform(Operation::Scale)),
            ]);
            if self
                .selection()
                .is_some_and(|o| matches!(o.kind, Kind::Box | Kind::Tank))
            {
                list.extend([
                    (
                        "Fluid Domain".into(),
                        Command::Role(scene::FluidRole::Domain),
                    ),
                    ("Fluid Flow".into(), Command::Role(scene::FluidRole::Flow)),
                    (
                        "Fluid Effector".into(),
                        Command::Role(scene::FluidRole::Effector),
                    ),
                ]);
            }
        }
        list.extend(
            mesh::Primitive::ALL
                .into_iter()
                .map(|p| (format!("Add {}", p.name()), Command::Add(p))),
        );
        list.extend(
            [
                Kind::Tank,
                Kind::Emitter,
                Kind::Drain,
                Kind::Field,
                Kind::Light,
                Kind::Camera,
            ]
            .into_iter()
            .map(|kind| (format!("Add {}", kind.label()), Command::Object(kind))),
        );
        list
    }

    pub(super) fn command_dialogs(&mut self, ctx: &egui::Context) {
        if self.search {
            let first = ctx.input(|i| i.key_pressed(Key::F3));
            let commands = self.commands();
            let mut command = None;
            let mut open = self.search;
            egui::Window::new("Operator Search")
                .open(&mut open)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 85.0))
                .default_width(340.0)
                .show(ctx, |ui| {
                    let result = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text("Search operators")
                            .desired_width(f32::INFINITY),
                    );
                    if first || !ui.memory(|m| m.has_focus(result.id)) {
                        result.request_focus();
                    }
                    if result.changed() {
                        self.search_cursor = 0;
                    }
                    let query = self.query.to_lowercase();
                    let matches: Vec<_> = commands
                        .iter()
                        .filter(|(label, _)| {
                            query
                                .split_whitespace()
                                .all(|word| label.to_lowercase().contains(word))
                        })
                        .collect();
                    if ctx.input(|i| i.key_pressed(Key::ArrowDown)) {
                        self.search_cursor =
                            (self.search_cursor + 1).min(matches.len().saturating_sub(1));
                    }
                    if ctx.input(|i| i.key_pressed(Key::ArrowUp)) {
                        self.search_cursor = self.search_cursor.saturating_sub(1);
                    }
                    if ctx.input(|i| i.key_pressed(Key::Enter)) {
                        command = matches.get(self.search_cursor).map(|(_, op)| *op);
                    }
                    egui::ScrollArea::vertical()
                        .max_height(280.0)
                        .show(ui, |ui| {
                            for (i, (label, op)) in matches.iter().enumerate() {
                                let row = ui.selectable_label(i == self.search_cursor, label);
                                if row.clicked() {
                                    command = Some(*op);
                                }
                                if i == self.search_cursor
                                    && ctx.input(|i| {
                                        i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::ArrowUp)
                                    })
                                {
                                    row.scroll_to_me(None);
                                }
                            }
                        });
                });
            if !open || command.is_some() || ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.search = false;
                ctx.memory_mut(|m| m.stop_text_input());
            }
            if let Some(command) = command {
                match command {
                    Command::Add(p) => self.add_primitive(p),
                    Command::Object(kind) => self.add_object(kind),
                    Command::Role(role) => {
                        if let Some(id) = self.selected {
                            let before = self.scene.clone();
                            self.scene.assign_role(id, role);
                            self.remember(before);
                            self.properties = Properties::Simulation;
                        }
                    }
                    Command::Transform(op) => {
                        self.transform = self
                            .selected
                            .and_then(|id| Transform::new(&self.scene, id, op))
                    }
                    Command::Edit => self.toggle_edit(),
                    Command::Frame => self.frame_scene(),
                    Command::Physics => self.properties = Properties::Simulation,
                    Command::LiveMove | Command::LiveRotate => {
                        self.live_tool = Some(if matches!(command, Command::LiveMove) {
                            Operation::Move
                        } else {
                            Operation::Rotate
                        });
                    }
                    Command::Split => {
                        self.area = ViewportArea::Scene;
                        self.split_view = !self.split_view;
                        self.dirty = true;
                    }
                }
            }
        }
        if self.adjust_last {
            let mut open = true;
            let mut done = false;
            if let Some(t) = &mut self.redo_transform {
                egui::Window::new("Adjust Last Operation")
                    .open(&mut open)
                    .collapsible(false)
                    .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(18.0, -96.0))
                    .show(ctx, |ui| {
                        let label = match t.operation {
                            Operation::Move => "Move",
                            Operation::Rotate => "Rotate",
                            Operation::Scale => "Scale",
                        };
                        ui.label(label);
                        ui.horizontal(|ui| {
                            for (axis, name) in ["Free", "X", "Y", "Z"].into_iter().enumerate() {
                                let value = axis.checked_sub(1);
                                self.dirty |=
                                    ui.selectable_value(&mut t.axis, value, name).changed();
                            }
                        });
                        ui.add(
                            egui::TextEdit::singleline(&mut t.number).hint_text("Numeric value"),
                        );
                        if ui.button("Done").clicked() {
                            done = true;
                        }
                    });
                if let Some(object) = self
                    .scene
                    .objects
                    .iter_mut()
                    .find(|o| o.id == t.original.id)
                {
                    let before = (object.position, object.rotation, object.half, object.radius);
                    t.apply(object, self.camera, ctx.available_rect().height());
                    self.dirty |=
                        before != (object.position, object.rotation, object.half, object.radius);
                }
            } else {
                done = true;
            }
            self.adjust_last = open && !done && !ctx.input(|i| i.key_pressed(Key::Escape));
        }
    }
}
