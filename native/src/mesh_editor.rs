use eframe::egui::{Pos2, Response};
use std::collections::BTreeSet;

use super::*;
use crate::mesh::{CutSide, LoopRing, Mesh, Primitive, edge, sides};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Select {
    Vertex,
    Edge,
    Face,
}

#[derive(Clone, Copy)]
enum Command {
    Inset,
    Bevel,
    Fill,
    Merge,
    Dissolve,
    Normals,
    Bisect,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SnapTarget {
    Grid,
    Vertex,
    Edge,
    Face,
}

enum Gesture {
    Extrude {
        faces: BTreeSet<usize>,
        normal: Vec3,
        axis: Option<usize>,
        delta: Vec2,
        number: String,
    },
    Loop {
        edge: Option<(u32, u32)>,
        ring: Option<LoopRing>,
        count: usize,
        slide: Option<(Pos2, f32)>,
    },
    Bevel {
        edges: BTreeSet<(u32, u32)>,
        width: f32,
        segments: usize,
    },
    Slide {
        vertices: BTreeSet<u32>,
        delta: Vec2,
    },
}

struct Drag {
    transform: Transform,
    pivot: Vec3,
    indices: BTreeSet<u32>,
}

type EdgePoint = ((u32, u32), f32);

pub struct Edit {
    id: u32,
    mode: Select,
    vertices: BTreeSet<u32>,
    edges: BTreeSet<(u32, u32)>,
    faces: BTreeSet<usize>,
    knife: bool,
    knife_start: Option<(usize, EdgePoint)>,
    drag: Option<Drag>,
    bisect: bool,
    side: CutSide,
    axis: usize,
    offset: f32,
    cap: bool,
    depth: f32,
    inset: f32,
    gesture: Option<Gesture>,
    preview: Option<Mesh>,
    slice: Option<(Pos2, Pos2)>,
    collision_preview: bool,
    snap_target: SnapTarget,
}

impl Edit {
    fn new(id: u32) -> Self {
        Self {
            id,
            mode: Select::Face,
            vertices: BTreeSet::new(),
            edges: BTreeSet::new(),
            faces: BTreeSet::new(),
            knife: false,
            knife_start: None,
            drag: None,
            bisect: false,
            side: CutSide::Both,
            axis: 2,
            offset: 0.0,
            cap: true,
            depth: 0.20,
            inset: 0.18,
            gesture: None,
            preview: None,
            slice: None,
            collision_preview: false,
            snap_target: SnapTarget::Grid,
        }
    }

    fn selected_vertices(&self, mesh: &Mesh) -> BTreeSet<u32> {
        match self.mode {
            Select::Vertex => self.vertices.clone(),
            Select::Edge => self.edges.iter().flat_map(|&(a, b)| [a, b]).collect(),
            Select::Face => self
                .faces
                .iter()
                .filter_map(|&i| mesh.faces.get(i))
                .flatten()
                .copied()
                .collect(),
        }
    }

    fn selected_faces(&self, mesh: &Mesh) -> BTreeSet<usize> {
        if self.mode == Select::Face {
            return self.faces.clone();
        }
        let vertices = self.selected_vertices(mesh);
        mesh.faces
            .iter()
            .enumerate()
            .filter_map(|(i, f)| f.iter().all(|v| vertices.contains(v)).then_some(i))
            .collect()
    }

    fn clear(&mut self) {
        self.vertices.clear();
        self.edges.clear();
        self.faces.clear();
        self.knife_start = None;
    }

    fn cancel_tool(&mut self) {
        self.gesture = None;
        self.preview = None;
        self.slice = None;
    }
}

impl App {
    pub(super) fn add_primitive(&mut self, kind: Primitive) {
        if self.scene.objects.len() >= MAX_OBJECTS {
            self.notice = "Scene object limit reached.".into();
            return;
        }
        if self.edit.is_some() {
            self.toggle_edit();
        }
        let before = self.scene.clone();
        let id = self.next_object_id();
        let half = kind.half();
        let mut object = Object::new(
            id,
            Kind::Box,
            if self.cursor_placed {
                self.cursor
            } else {
                Vec3::new(
                    self.camera.pivot.x.clamp(-1.3, 1.3),
                    half.y + 0.18,
                    self.camera.pivot.z.clamp(-0.8, 0.8),
                )
            },
        );
        object.name = kind.name().into();
        object.half = half;
        object.mesh = Some(kind.geometry());
        self.scene.objects.push(object);
        if !self.scene.within_mesh_budget() {
            self.scene = before;
            self.notice = "Scene mesh limit reached.".into();
            return;
        }
        self.remember(before);
        self.selected = Some(id);
        self.add = false;
        self.edit = None;
    }

    pub(super) fn toggle_edit(&mut self) {
        if let Some(edit) = self.edit.take() {
            if let Some(drag) = edit.drag {
                self.scene = drag.transform.before;
            }
            self.dirty = true;
            return;
        }
        let Some(id) = self.selected else {
            self.notice = "Select a collider to enter Edit Mode.".into();
            return;
        };
        let Some(i) = self
            .scene
            .objects
            .iter()
            .position(|o| o.id == id && matches!(o.kind, Kind::Box | Kind::Tank))
        else {
            self.notice = "Select a mesh, solid box, or tank to edit.".into();
            return;
        };
        self.show_frame(None);
        self.playing = false;
        self.step_once = false;
        self.rock = None;
        self.transform = None;
        if self.scene.objects[i].mesh.is_none() {
            let before = self.scene.clone();
            let object = &mut self.scene.objects[i];
            let tank = object.kind == Kind::Tank;
            object.mesh = Some(if tank {
                Mesh::tank_shell(object.half)
            } else {
                Primitive::Cube.geometry()
            });
            object.mesh_glass = tank;
            object.kind = Kind::Box;
            if !self.scene.within_mesh_budget() {
                self.scene = before;
                self.notice = "Scene mesh limit reached.".into();
                return;
            }
            self.remember(before);
        }
        self.edit = Some(Edit::new(id));
        self.properties = Properties::Object;
        self.dirty = true;
    }

    fn mesh_action(&mut self, command: Command) {
        let Some(edit) = &mut self.edit else {
            return;
        };
        let Some(i) = self.scene.objects.iter().position(|o| o.id == edit.id) else {
            return;
        };
        let Some(original) = self.scene.objects[i].mesh.as_ref() else {
            return;
        };
        let before = self.scene.clone();
        let mut mesh = original.clone();
        let selected = edit.selected_vertices(&mesh);
        let faces = edit.selected_faces(&mesh);
        let changed = match command {
            Command::Inset | Command::Bevel => {
                let height = if matches!(command, Command::Bevel) {
                    edit.depth * 0.35
                } else {
                    0.0
                };
                let mut changed = false;
                for face in faces {
                    changed |= mesh.inset(face, edit.inset, height).is_some();
                }
                changed
            }
            Command::Fill => mesh.fill(&selected),
            Command::Merge => mesh.merge(&selected),
            Command::Dissolve => mesh.dissolve(&selected),
            Command::Normals => {
                mesh.recalculate();
                true
            }
            Command::Bisect => {
                mesh.bisect(input::AXES[edit.axis], edit.offset, edit.side, edit.cap)
            }
        };
        if !changed {
            self.notice = "Select suitable mesh elements for this tool.".into();
            return;
        }
        if !mesh.valid() {
            self.notice = "The edit would exceed the mesh limits or leave empty geometry.".into();
            return;
        }
        mesh.recalculate();
        self.scene.objects[i].mesh = Some(mesh);
        if !self.scene.within_mesh_budget() {
            self.scene = before;
            self.notice = "Scene mesh limit reached.".into();
            return;
        }
        if matches!(
            command,
            Command::Bisect | Command::Merge | Command::Dissolve
        ) {
            edit.clear();
        }
        edit.bisect = false;
        self.notice.clear();
        self.remember(before);
    }

    pub(super) fn edit_keys(&mut self, ctx: &egui::Context) {
        let Some(edit) = &mut self.edit else {
            return;
        };
        let mods = ctx.input(|i| i.modifiers);
        let key = |k| ctx.input(|i| i.key_pressed(k));
        if let Some(drag) = &edit.drag {
            if key(Key::G)
                && !mods.command
                && matches!(drag.transform.operation, Operation::Move)
                && matches!(edit.mode, Select::Edge | Select::Vertex)
            {
                let drag = edit.drag.take().unwrap();
                self.scene = drag.transform.before;
                edit.gesture = Some(Gesture::Slide {
                    vertices: drag.indices,
                    delta: Vec2::ZERO,
                });
            }
            return;
        }
        if edit.gesture.is_some() {
            return;
        }
        if key(Key::Escape) {
            edit.knife = false;
            edit.knife_start = None;
            edit.bisect = false;
            edit.cancel_tool();
            return;
        }
        if mods.command && key(Key::Z) {
            edit.clear();
            self.history(mods.shift);
            return;
        }
        if mods.ctrl && key(Key::R) {
            self.start_loop();
            return;
        }
        if mods.ctrl && key(Key::B) {
            self.start_bevel();
            return;
        }
        if mods.shift && key(Key::N) {
            self.mesh_action(Command::Normals);
            return;
        }
        if mods.command || mods.shift || mods.alt {
            return;
        }
        for (keycode, mode) in [
            (Key::Num1, Select::Vertex),
            (Key::Num2, Select::Edge),
            (Key::Num3, Select::Face),
        ] {
            if key(keycode) && edit.mode != mode {
                edit.mode = mode;
                edit.clear();
            }
        }
        if key(Key::K) {
            edit.knife = !edit.knife;
            edit.knife_start = None;
        }
        if key(Key::E) {
            self.start_extrude();
            return;
        }
        if key(Key::A)
            && let Some(mesh) = self
                .scene
                .objects
                .iter()
                .find(|o| o.id == edit.id)
                .and_then(|o| o.mesh.as_ref())
        {
            match edit.mode {
                Select::Vertex => edit.vertices = (0..mesh.points.len() as u32).collect(),
                Select::Edge => edit.edges = mesh.edges(),
                Select::Face => edit.faces = (0..mesh.faces.len()).collect(),
            }
        }
        for (k, op) in [
            (Key::G, Operation::Move),
            (Key::R, Operation::Rotate),
            (Key::S, Operation::Scale),
        ] {
            if key(k) {
                let Some(object) = self.scene.objects.iter().find(|o| o.id == edit.id) else {
                    return;
                };
                let Some(mesh) = &object.mesh else {
                    return;
                };
                let indices = edit.selected_vertices(mesh);
                if indices.is_empty() {
                    return;
                }
                let pivot = indices
                    .iter()
                    .map(|&i| world(object, mesh.points[i as usize]))
                    .sum::<Vec3>()
                    / indices.len() as f32;
                let mut transform = Transform::new(&self.scene, object.id, op).unwrap();
                transform.original.position = pivot;
                transform.original.half = Vec3::ONE;
                transform.mesh_elements = true;
                edit.drag = Some(Drag {
                    transform,
                    pivot,
                    indices,
                });
                return;
            }
        }
        let command = [
            (Key::I, Command::Inset),
            (Key::F, Command::Fill),
            (Key::M, Command::Merge),
            (Key::X, Command::Dissolve),
        ]
        .into_iter()
        .find(|(k, _)| key(*k))
        .map(|(_, c)| c);
        if let Some(command) = command {
            self.mesh_action(command);
        }
    }

    fn start_loop(&mut self) {
        if let Some(edit) = &mut self.edit
            && edit.drag.is_none()
        {
            edit.cancel_tool();
            edit.knife = false;
            edit.bisect = false;
            edit.gesture = Some(Gesture::Loop {
                edge: None,
                ring: None,
                count: 1,
                slide: None,
            });
        }
    }

    fn start_extrude(&mut self) {
        let Some(edit) = &mut self.edit else {
            return;
        };
        if edit.drag.is_some() {
            return;
        }
        let Some(object) = self.scene.objects.iter().find(|o| o.id == edit.id) else {
            return;
        };
        let Some(mesh) = &object.mesh else {
            return;
        };
        let faces = edit.selected_faces(mesh);
        let normal = faces
            .iter()
            .map(|&i| mesh.normal(&mesh.faces[i]))
            .sum::<Vec3>();
        let normal = (object.rotation * (normal / object.half)).normalize_or_zero();
        if normal.length_squared() < 0.5 {
            self.notice = "Select a face or a region with a shared direction.".into();
            return;
        }
        edit.cancel_tool();
        edit.knife = false;
        edit.bisect = false;
        edit.gesture = Some(Gesture::Extrude {
            faces,
            normal,
            axis: None,
            delta: Vec2::ZERO,
            number: String::new(),
        });
    }

    fn start_bevel(&mut self) {
        let Some(edit) = &mut self.edit else {
            return;
        };
        if edit.drag.is_some() {
            return;
        }
        if edit.edges.is_empty() {
            self.notice = "Select an edge shared by two faces.".into();
            return;
        }
        let edges = edit.edges.clone();
        edit.cancel_tool();
        edit.knife = false;
        edit.bisect = false;
        edit.gesture = Some(Gesture::Bevel {
            edges,
            width: 0.08,
            segments: 1,
        });
    }

    pub(super) fn mesh_panel(&mut self, ui: &mut egui::Ui) {
        let Some(edit) = &mut self.edit else {
            return;
        };
        ui.horizontal(|ui| {
            for (m, name) in [
                (Select::Vertex, "Vertex"),
                (Select::Edge, "Edge"),
                (Select::Face, "Face"),
            ] {
                if ui.selectable_label(edit.mode == m, name).clicked() {
                    edit.mode = m;
                    edit.clear();
                }
            }
        });
        let mut action = None;
        let mut start_tool = None;
        slider(ui, "Depth", &mut edit.depth, -1.0..=1.0, 2);
        slider(ui, "Inset", &mut edit.inset, 0.02..=0.75, 2);
        ui.checkbox(&mut edit.collision_preview, "Collision preview");
        ui.horizontal_wrapped(|ui| {
            ui.label("Snap to");
            for (target, label) in [
                (SnapTarget::Grid, "Grid"),
                (SnapTarget::Vertex, "Vertex"),
                (SnapTarget::Edge, "Edge"),
                (SnapTarget::Face, "Face"),
            ] {
                ui.selectable_value(&mut edit.snap_target, target, label);
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (i, label, key) in [
                (0, "Extrude", "E"),
                (1, "Loop Cut", "Ctrl+R"),
                (2, "Bevel Edge", "Ctrl+B"),
            ] {
                if ui.button(label).on_hover_text(key).clicked() {
                    start_tool = Some(i);
                }
            }
            for (c, label) in [
                (Command::Inset, "Inset"),
                (Command::Bevel, "Bevel rim"),
                (Command::Fill, "Fill"),
                (Command::Merge, "Merge"),
                (Command::Dissolve, "Dissolve"),
                (Command::Normals, "Recalc normals"),
            ] {
                if ui.button(label).clicked() {
                    action = Some(c);
                }
            }
            if ui
                .selectable_label(edit.knife, "Knife")
                .on_hover_text("K · click two edges on a face")
                .clicked()
            {
                edit.knife = !edit.knife;
                edit.knife_start = None;
                edit.cancel_tool();
                edit.bisect = false;
            }
            if ui.selectable_label(edit.bisect, "Bisect").clicked() {
                edit.cancel_tool();
                edit.bisect = !edit.bisect;
                edit.knife = false;
            }
        });
        if edit.bisect {
            ui.separator();
            ui.label("Local cutting plane");
            ui.label("Drag a line in the viewport or set a local plane here.");
            ui.horizontal(|ui| {
                for (i, name) in ["X", "Y", "Z"].into_iter().enumerate() {
                    ui.selectable_value(&mut edit.axis, i, name);
                }
            });
            slider(ui, "Offset", &mut edit.offset, -4.0..=4.0, 2);
            ui.horizontal_wrapped(|ui| {
                for (side, name) in [
                    (CutSide::Both, "Keep both"),
                    (CutSide::Positive, "Positive"),
                    (CutSide::Negative, "Negative"),
                ] {
                    ui.selectable_value(&mut edit.side, side, name);
                }
            });
            ui.add_enabled(
                edit.side != CutSide::Both,
                egui::Checkbox::new(&mut edit.cap, "Fill cut"),
            );
            if ui.button("Apply cut").clicked() {
                action = Some(Command::Bisect);
            }
        }
        if let Some(command) = action {
            self.mesh_action(command);
        }
        match start_tool {
            Some(0) => self.start_extrude(),
            Some(1) => self.start_loop(),
            Some(2) => self.start_bevel(),
            _ => {}
        }
    }

    pub(super) fn edit_pointer(
        &mut self,
        ctx: &egui::Context,
        rect: egui::Rect,
        response: &Response,
    ) {
        let Some(edit) = &mut self.edit else {
            return;
        };
        let Some(index) = self.scene.objects.iter().position(|o| o.id == edit.id) else {
            return;
        };
        if edit.gesture.is_some() {
            self.mesh_gesture(ctx, rect, response, index);
            return;
        }
        if edit.bisect {
            self.viewport_slice(ctx, rect, response, index);
            return;
        }
        if let Some(drag) = &mut edit.drag {
            input::read_transform(&mut drag.transform, ctx);
            let snap = self.snapping || ctx.input(|i| i.modifiers.ctrl);
            drag.transform.snap = snap && edit.snap_target == SnapTarget::Grid;
            if response.hovered() {
                drag.transform.delta += ctx.input(|i| i.pointer.delta());
            }
            let mut basis = drag.transform.original.clone();
            drag.transform.apply(&mut basis, self.camera, rect.height());
            if snap
                && drag.transform.number.is_empty()
                && matches!(drag.transform.operation, Operation::Move)
                && let Some(cursor) = ctx.pointer_hover_pos()
                && let Some(target) = snap_point(
                    &self.scene,
                    edit.id,
                    &drag.indices,
                    edit.snap_target,
                    self.camera,
                    rect,
                    cursor,
                )
            {
                let offset = target - drag.pivot;
                basis.position = if let Some(axis) = drag.transform.axis {
                    let mut dir = input::AXES[axis];
                    if drag.transform.local {
                        dir = drag.transform.original.rotation * dir;
                    }
                    drag.pivot + dir * offset.dot(dir)
                } else {
                    target
                };
            }
            let current = &mut self.scene.objects[index];
            let Some(old) = drag
                .transform
                .before
                .objects
                .iter()
                .find(|o| o.id == edit.id)
                .and_then(|o| o.mesh.as_ref())
            else {
                return;
            };
            let Some(mesh) = &mut current.mesh else {
                return;
            };
            let (position, rotation, half) = (current.position, current.rotation, current.half);
            let turn = basis.rotation * drag.transform.original.rotation.conjugate();
            for &i in &drag.indices {
                let point = position + rotation * (old.points[i as usize] * half);
                let offset = point - drag.pivot;
                let scaled = if drag.transform.local {
                    rotation * ((rotation.conjugate() * offset) * basis.half)
                } else {
                    offset * basis.half
                };
                let point = basis.position + turn * scaled;
                mesh.points[i as usize] = (rotation.conjugate() * (point - position) / half)
                    .clamp(Vec3::splat(-32.0), Vec3::splat(32.0));
            }
            self.dirty = true;
            if ctx.input(|i| i.key_pressed(Key::Escape))
                || response.clicked_by(PointerButton::Secondary)
            {
                self.scene = edit.drag.take().unwrap().transform.before;
            } else if ctx.input(|i| i.key_pressed(Key::Enter))
                || response.clicked_by(PointerButton::Primary)
            {
                let before = edit.drag.take().unwrap().transform.before;
                self.remember(before);
            }
            return;
        }
        if !response.clicked_by(PointerButton::Primary) || ctx.input(|i| i.modifiers.alt) {
            return;
        }
        let Some(pointer) = response.interact_pointer_pos() else {
            return;
        };
        let object = &self.scene.objects[index];
        let Some(mesh) = object.mesh.as_ref() else {
            return;
        };
        let view = Projected::new(object, mesh, self.camera, rect);
        let picked = view.face(mesh, pointer);
        if edit.knife {
            let face = edit
                .knife_start
                .map(|(f, _)| f)
                .or(picked)
                .or_else(|| view.closest_face_edge(mesh, pointer));
            let Some(face) = face else {
                return;
            };
            let Some(hit) = view.edge(mesh, Some(face), pointer) else {
                return;
            };
            if let Some((old_face, first)) = edit.knife_start.take() {
                let before = self.scene.clone();
                let mut new = mesh.clone();
                if new.knife(old_face, [first, hit]) && new.valid() {
                    self.scene.objects[index].mesh = Some(new);
                    if self.scene.within_mesh_budget() {
                        edit.knife = false;
                        edit.faces = BTreeSet::from([old_face]);
                        self.remember(before);
                    } else {
                        self.scene = before;
                    }
                }
            } else {
                edit.knife_start = Some((face, hit));
            }
            return;
        }
        if !ctx.input(|i| i.modifiers.shift) {
            edit.clear();
        }
        match edit.mode {
            Select::Vertex => {
                if let Some(v) = view.vertex(mesh, picked, pointer) {
                    toggle(&mut edit.vertices, v);
                }
            }
            Select::Edge => {
                if let Some((e, _)) = view.edge(mesh, picked, pointer) {
                    toggle(&mut edit.edges, edge(e.0, e.1));
                }
            }
            Select::Face => {
                if let Some(f) = picked {
                    toggle(&mut edit.faces, f);
                }
            }
        }
    }

    pub(super) fn edit_scroll_owned(&self) -> bool {
        self.edit
            .as_ref()
            .and_then(|e| e.gesture.as_ref())
            .is_some_and(|g| matches!(g, Gesture::Loop { .. } | Gesture::Bevel { .. }))
    }

    fn mesh_gesture(
        &mut self,
        ctx: &egui::Context,
        rect: egui::Rect,
        response: &Response,
        index: usize,
    ) {
        let edit = self.edit.as_mut().unwrap();
        let key = |k| ctx.input(|i| i.key_pressed(k));
        let right = response.clicked_by(PointerButton::Secondary);
        let left = response.clicked_by(PointerButton::Primary);
        let cursor = ctx.pointer_hover_pos().filter(|p| rect.contains(*p));
        let delta = if response.hovered() {
            ctx.input(|i| i.pointer.delta())
        } else {
            Vec2::ZERO
        };
        let scroll = if response.hovered() {
            ctx.input(|i| i.raw_scroll_delta.y)
        } else {
            0.0
        };
        let snap = self.snapping || ctx.input(|i| i.modifiers.ctrl);
        let object = &self.scene.objects[index];
        let Some(mesh) = &object.mesh else {
            edit.cancel_tool();
            return;
        };
        let gesture = edit.gesture.as_mut().unwrap();
        if key(Key::Escape) || (right && !matches!(gesture, Gesture::Loop { slide: Some(_), .. })) {
            edit.cancel_tool();
            return;
        }
        let mut result = None;
        let mut selection = None;
        match gesture {
            Gesture::Extrude {
                faces,
                normal,
                axis,
                delta: movement,
                number,
            } => {
                for (k, a) in [(Key::X, 0), (Key::Y, 1), (Key::Z, 2)] {
                    if key(k) {
                        *axis = if *axis == Some(a) { None } else { Some(a) };
                    }
                }
                if key(Key::Backspace) {
                    number.pop();
                }
                ctx.input(|i| {
                    for e in &i.events {
                        if let egui::Event::Text(s) = e {
                            number.extend(
                                s.chars()
                                    .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                                    .take(12usize.saturating_sub(number.len())),
                            );
                        }
                    }
                });
                *movement += delta;
                let dir = axis.map_or(*normal, |a| input::AXES[a]);
                let (_, horizontal, vertical, _) = self.camera.basis();
                let projected = Vec2::new(dir.dot(horizontal), -dir.dot(vertical));
                let projected = if projected.length_sq() > 0.015 {
                    projected.normalized()
                } else {
                    Vec2::UP
                };
                let distance = number
                    .parse::<f32>()
                    .ok()
                    .filter(|x| x.is_finite())
                    .unwrap_or_else(|| {
                        let v =
                            movement.dot(projected) * self.camera.distance / rect.height().max(1.0);
                        if snap { (v / 0.10).round() * 0.10 } else { v }
                    })
                    .clamp(-3.0, 3.0);
                let mut new = mesh.clone();
                if distance.abs() > 0.0001
                    && new.extrude(
                        faces,
                        object.rotation.conjugate() * (dir * distance) / object.half,
                    )
                {
                    new.recalculate();
                    edit.preview = Some(new);
                } else {
                    edit.preview = None;
                }
                if left || key(Key::Enter) {
                    result = edit.preview.take();
                }
            }
            Gesture::Loop {
                edge: current,
                ring,
                count,
                slide,
            } => {
                if let Some((anchor, amount)) = slide {
                    if let Some(cursor) = cursor {
                        let direction = current
                            .and_then(|(a, b)| {
                                let view = Projected::new(object, mesh, self.camera, rect);
                                Some(view.points[b as usize]? - view.points[a as usize]?)
                            })
                            .filter(|d| d.length_sq() > 0.001)
                            .map(|d| d.normalized())
                            .unwrap_or(Vec2::RIGHT);
                        *amount = ((cursor - *anchor).dot(direction) / 110.0).clamp(-0.95, 0.95);
                    }
                    if left || right || key(Key::Enter) {
                        let mut new = mesh.clone();
                        if let Some(ring) = ring
                            && let Some(edges) =
                                new.cut_ring(ring, *count, if right { 0.0 } else { *amount })
                        {
                            result = Some(new);
                            selection = Some(edges);
                        }
                    }
                } else {
                    if scroll != 0.0 {
                        *count = (*count as i32 + scroll.signum() as i32).clamp(1, 8) as usize;
                    }
                    if let Some(cursor) = cursor {
                        let view = Projected::new(object, mesh, self.camera, rect);
                        let candidate = view
                            .edge(mesh, view.face(mesh, cursor), cursor)
                            .map(|(e, _)| e);
                        if *current != candidate {
                            *current = candidate;
                            *ring = candidate.and_then(|e| mesh.loop_ring(e));
                        }
                        if ring.is_some() && (left || key(Key::Enter)) {
                            *slide = Some((cursor, 0.0));
                        }
                    }
                }
            }
            Gesture::Bevel {
                edges,
                width,
                segments,
            } => {
                *width = (*width
                    + delta.x * self.camera.distance
                        / (rect.height().max(1.0) * object.half.max_element()))
                .clamp(0.001, 1.0);
                if scroll != 0.0 {
                    *segments = (*segments as i32 + scroll.signum() as i32).clamp(1, 8) as usize;
                }
                let mut used = BTreeSet::new();
                let mut new = mesh.clone();
                let changed = edges.iter().all(|&(a, b)| {
                    used.insert(a) && used.insert(b) && new.bevel_edge((a, b), *width, *segments)
                });
                edit.preview = changed.then_some(new);
                if left || key(Key::Enter) {
                    result = edit.preview.take();
                    if result.is_none() {
                        self.notice = "Select separate edges shared by two angled faces.".into();
                    }
                }
            }
            Gesture::Slide {
                vertices,
                delta: movement,
            } => {
                *movement += delta;
                let view = Projected::new(object, mesh, self.camera, rect);
                let mut new = mesh.clone();
                let edges = mesh.edges();
                for &a in vertices.iter() {
                    let Some(origin) = view.points[a as usize] else {
                        continue;
                    };
                    let candidate = edges
                        .iter()
                        .filter_map(|&(u, v)| {
                            let b = if u == a {
                                v
                            } else if v == a {
                                u
                            } else {
                                return None;
                            };
                            if vertices.contains(&b) {
                                return None;
                            }
                            let d = view.points[b as usize]? - origin;
                            (d.length_sq() > 4.0).then_some((b, d, movement.dot(d.normalized())))
                        })
                        .filter(|(_, _, amount)| *amount > 0.0)
                        .max_by(|a, b| a.2.total_cmp(&b.2));
                    if let Some((b, d, amount)) = candidate {
                        new.points[a as usize] = mesh.points[a as usize]
                            .lerp(mesh.points[b as usize], (amount / d.length()).min(0.98));
                    }
                }
                edit.preview = Some(new);
                if left || key(Key::Enter) {
                    result = edit.preview.take();
                }
            }
        }
        if let Some(new) = result {
            edit.cancel_tool();
            let before = self.scene.clone();
            if !new.valid() {
                self.notice = "The edit exceeds the mesh limits.".into();
                return;
            }
            self.scene.objects[index].mesh = Some(new);
            if !self.scene.within_mesh_budget() {
                self.scene = before;
                self.notice = "Scene mesh limit reached.".into();
                return;
            }
            if let Some(edges) = selection {
                edit.clear();
                edit.mode = Select::Edge;
                edit.edges = edges;
            }
            self.notice.clear();
            self.remember(before);
        }
    }

    fn viewport_slice(
        &mut self,
        ctx: &egui::Context,
        rect: egui::Rect,
        response: &Response,
        index: usize,
    ) {
        let edit = self.edit.as_mut().unwrap();
        if ctx.input(|i| i.key_pressed(Key::Escape))
            || response.clicked_by(PointerButton::Secondary)
        {
            edit.slice = None;
            edit.bisect = false;
            return;
        }
        if let Some(cursor) = ctx.pointer_interact_pos() {
            if response.drag_started_by(PointerButton::Primary) && !ctx.input(|i| i.modifiers.alt) {
                edit.slice = Some((cursor - response.drag_delta(), cursor));
            }
            if let Some((_, end)) = &mut edit.slice {
                *end = cursor;
            }
        }
        if !response.drag_stopped_by(PointerButton::Primary) {
            return;
        }
        let Some((a, b)) = edit.slice.take() else {
            return;
        };
        if a.distance(b) < 7.0 {
            return;
        }
        let ray = |p: Pos2| {
            let uv = (p - rect.min) / rect.size();
            self.camera
                .ray(glam::Vec2::new(uv.x, uv.y), rect.width() / rect.height())
        };
        let (origin, first) = ray(a);
        let (other, second) = ray(b);
        let world_normal = if self.camera.ortho {
            (other - origin).cross(first)
        } else {
            second.cross(first)
        }
        .normalize_or_zero();
        let object = &self.scene.objects[index];
        let normal =
            (object.half * (object.rotation.conjugate() * world_normal)).normalize_or_zero();
        if normal.length_squared() < 0.5 {
            return;
        }
        let point = object.rotation.conjugate() * (origin - object.position) / object.half;
        let Some(mut mesh) = object.mesh.clone() else {
            return;
        };
        if !mesh.bisect(normal, normal.dot(point), edit.side, edit.cap) || !mesh.valid() {
            return;
        }
        let before = self.scene.clone();
        self.scene.objects[index].mesh = Some(mesh);
        if self.scene.within_mesh_budget() {
            edit.clear();
            edit.bisect = false;
            self.remember(before);
        } else {
            self.scene = before;
            self.notice = "Scene mesh limit reached.".into();
        }
    }

    pub(super) fn mesh_overlay(&self, rect: egui::Rect, painter: &egui::Painter) {
        let Some(edit) = &self.edit else {
            return;
        };
        let Some(object) = self.scene.objects.iter().find(|o| o.id == edit.id) else {
            return;
        };
        let Some(mesh) = &object.mesh else {
            return;
        };
        let mesh = edit.preview.as_ref().unwrap_or(mesh);
        let view = Projected::new(object, mesh, self.camera, rect);
        let selected = edit.selected_vertices(mesh);
        let amber = Color32::from_rgb(255, 178, 96);
        let mut edge_counts = std::collections::BTreeMap::new();
        for (a, b) in mesh.faces.iter().flat_map(|face| sides(face)) {
            *edge_counts.entry(edge(a, b)).or_insert(0u32) += 1;
        }
        let closed = edge_counts.values().all(|count| *count == 2);
        for ((a, b), count) in edge_counts {
            if let (Some(p), Some(q)) = (view.points[a as usize], view.points[b as usize]) {
                let active = edit.edges.contains(&edge(a, b))
                    || (selected.contains(&a) && selected.contains(&b));
                painter.line_segment(
                    [p, q],
                    Stroke::new(
                        if active { 2.2_f32 } else { 1.0_f32 },
                        if edit.collision_preview && count != 2 {
                            Color32::from_rgb(255, 103, 98)
                        } else if active {
                            amber
                        } else if edit.collision_preview {
                            ACCENT
                        } else {
                            Color32::from_rgb(37, 80, 95)
                        },
                    ),
                );
            }
        }
        if edit.collision_preview && !closed {
            for face in &mesh.faces {
                let normal =
                    (object.rotation * (mesh.normal(face) / object.half)).normalize_or_zero();
                for (a, b) in sides(face) {
                    for sign in [-1.0, 1.0] {
                        let edge = [a, b].map(|v| {
                            input::project(
                                self.camera,
                                rect,
                                world(object, mesh.points[v as usize])
                                    + normal * sign * object.thickness * 0.5,
                            )
                        });
                        if let [Some(a), Some(b)] = edge {
                            painter.line_segment(
                                [a, b],
                                Stroke::new(
                                    1.0_f32,
                                    Color32::from_rgba_unmultiplied(105, 230, 219, 95),
                                ),
                            );
                        }
                    }
                }
            }
        }
        if let Some(Gesture::Loop {
            ring: Some(ring),
            count,
            slide,
            ..
        }) = &edit.gesture
        {
            for line in ring.segments(mesh, *count, slide.map_or(0.0, |(_, v)| v)) {
                if let [Some(a), Some(b)] =
                    line.map(|p| input::project(self.camera, rect, world(object, p)))
                {
                    painter.line_segment([a, b], Stroke::new(2.5_f32, amber));
                }
            }
        }
        if edit.mode == Select::Vertex {
            for (i, p) in view.points.iter().enumerate() {
                if let Some(p) = p {
                    painter.circle_filled(
                        *p,
                        3.0,
                        if selected.contains(&(i as u32)) {
                            amber
                        } else {
                            ACCENT
                        },
                    );
                }
            }
        }
        if let Some((_, ((a, b), t))) = edit.knife_start
            && let (Some(p), Some(q)) = (view.points[a as usize], view.points[b as usize])
        {
            painter.circle_filled(p.lerp(q, t), 5.0, amber);
        }
        if let Some((a, b)) = edit.slice {
            painter.line_segment([a, b], Stroke::new(2.5_f32, amber));
            let side = Vec2::new(-(b - a).y, (b - a).x).normalized();
            if edit.side != CutSide::Both {
                let discard = if edit.side == CutSide::Positive {
                    -side
                } else {
                    side
                };
                painter.arrow(
                    a.lerp(b, 0.5),
                    discard * 24.0,
                    Stroke::new(1.7_f32, Color32::from_rgb(255, 103, 98)),
                );
            }
        } else if edit.bisect {
            let n = input::AXES[edit.axis];
            let hint = if n.x.abs() > 0.5 { Vec3::Y } else { Vec3::X };
            let y = n.cross(hint);
            let c = n * edit.offset;
            let vertices = [
                c - hint * 2.0 - y * 2.0,
                c + hint * 2.0 - y * 2.0,
                c + hint * 2.0 + y * 2.0,
                c - hint * 2.0 + y * 2.0,
            ];
            let corners: Vec<_> = vertices
                .into_iter()
                .filter_map(|p| input::project(self.camera, rect, world(object, p)))
                .collect();
            if corners.len() == 4 {
                painter.add(egui::Shape::convex_polygon(
                    corners,
                    Color32::from_rgba_unmultiplied(85, 196, 233, 45),
                    Stroke::new(1.5_f32, ACCENT),
                ));
            }
        }
        let label = if let Some(gesture) = &edit.gesture {
            match gesture {
                Gesture::Extrude { axis, number, .. } => format!(
                    "Extrude {} {number}  ·  XYZ constrain  ·  Click confirm  ·  Esc cancel",
                    axis.map_or("normal", |a| ["X", "Y", "Z"][a])
                ),
                Gesture::Loop {
                    count, slide: None, ..
                } => format!("Loop Cut  ·  {count} cuts  ·  Wheel count  ·  Click edge"),
                Gesture::Loop { slide: Some(_), .. } => {
                    "Slide cuts  ·  Click confirm  ·  Right-click center  ·  Esc cancel".into()
                }
                Gesture::Bevel { segments, .. } => {
                    format!("Bevel  ·  {segments} segments  ·  Wheel segments  ·  Click confirm")
                }
                Gesture::Slide { .. } => "Edge Slide  ·  Click confirm  ·  Esc cancel".into(),
            }
        } else if let Some(drag) = &edit.drag {
            drag.transform.label()
        } else if edit.collision_preview {
            if closed {
                "Collision preview · Closed solid".into()
            } else {
                format!(
                    "Collision preview · Open edges in red · {:.3} m shell",
                    object.thickness
                )
            }
        } else if edit.knife {
            "Knife · click two edges of one face".into()
        } else {
            "Edit Mode · Tab to return to Object Mode".into()
        };
        painter.text(
            rect.left_bottom() + Vec2::new(18.0, -18.0),
            egui::Align2::LEFT_BOTTOM,
            label,
            egui::FontId::proportional(12.0),
            amber,
        );
    }
}

fn toggle<T: Ord + Copy>(set: &mut BTreeSet<T>, item: T) {
    if !set.remove(&item) {
        set.insert(item);
    }
}

fn world(o: &Object, p: Vec3) -> Vec3 {
    o.position + o.rotation * (p * o.half)
}

fn snap_point(
    scene: &Scene,
    id: u32,
    selected: &BTreeSet<u32>,
    target: SnapTarget,
    camera: Camera,
    rect: egui::Rect,
    cursor: Pos2,
) -> Option<Vec3> {
    if target == SnapTarget::Grid {
        return None;
    }
    let mut best: Option<(f32, Vec3)> = None;
    let uv = (cursor - rect.min) / rect.size();
    let (origin, ray) = camera.ray(glam::Vec2::new(uv.x, uv.y), rect.width() / rect.height());
    for object in scene
        .objects
        .iter()
        .filter(|o| o.visible && o.enabled && matches!(o.kind, Kind::Box | Kind::Tank))
    {
        let fallback;
        let mesh = if let Some(mesh) = &object.mesh {
            mesh
        } else {
            fallback = if object.kind == Kind::Tank {
                Mesh::tank_shell(object.half)
            } else {
                Primitive::Cube.geometry()
            };
            &fallback
        };
        let available = |i: &u32| object.id != id || !selected.contains(i);
        let view = Projected::new(object, mesh, camera, rect);
        let candidate = match target {
            SnapTarget::Vertex => view
                .points
                .iter()
                .enumerate()
                .filter(|(i, _)| available(&(*i as u32)))
                .filter_map(|(i, p)| {
                    Some((p.as_ref()?.distance(cursor), world(object, mesh.points[i])))
                })
                .filter(|(d, _)| *d <= 18.0)
                .min_by(|a, b| a.0.total_cmp(&b.0)),
            SnapTarget::Edge => view
                .nearest_edge(
                    mesh.edges()
                        .into_iter()
                        .filter(|(a, b)| available(a) && available(b)),
                    cursor,
                )
                .and_then(|(d, ((a, b), t))| {
                    (d <= 18.0).then(|| {
                        (
                            d,
                            world(
                                object,
                                mesh.points[a as usize].lerp(mesh.points[b as usize], t),
                            ),
                        )
                    })
                }),
            SnapTarget::Face => view
                .face(mesh, cursor)
                .filter(|&f| mesh.faces[f].iter().all(available))
                .and_then(|f| {
                    let n = (object.rotation * (mesh.normal(&mesh.faces[f]) / object.half))
                        .normalize_or_zero();
                    let point = world(object, mesh.center(&mesh.faces[f]));
                    let denominator = n.dot(ray);
                    if denominator.abs() < 0.0001 {
                        return None;
                    }
                    let t = n.dot(point - origin) / denominator;
                    (t > 0.0).then_some((t, origin + ray * t))
                }),
            SnapTarget::Grid => None,
        };
        if let Some(candidate) = candidate
            && best.is_none_or(|(d, _)| candidate.0 < d)
        {
            best = Some(candidate);
        }
    }
    best.map(|(_, p)| p)
}

struct Projected {
    points: Vec<Option<Pos2>>,
    depths: Vec<f32>,
}

impl Projected {
    fn new(o: &Object, mesh: &Mesh, camera: Camera, rect: egui::Rect) -> Self {
        let (eye, _, _, forward) = camera.basis();
        Self {
            points: mesh
                .points
                .iter()
                .map(|&p| input::project(camera, rect, world(o, p)))
                .collect(),
            depths: mesh
                .points
                .iter()
                .map(|&p| (world(o, p) - eye).dot(forward))
                .collect(),
        }
    }

    fn face(&self, mesh: &Mesh, p: Pos2) -> Option<usize> {
        let mut best = None;
        let mut nearest = f32::INFINITY;
        for (i, f) in mesh.faces.iter().enumerate() {
            let mut inside = false;
            let mut visible = true;
            for (a, b) in sides(f) {
                let (Some(a), Some(b)) = (self.points[a as usize], self.points[b as usize]) else {
                    visible = false;
                    break;
                };
                if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x
                {
                    inside = !inside;
                }
            }
            if visible && inside {
                let depth =
                    f.iter().map(|&i| self.depths[i as usize]).sum::<f32>() / f.len() as f32;
                if depth < nearest {
                    nearest = depth;
                    best = Some(i);
                }
            }
        }
        best
    }

    fn nearest_edge(
        &self,
        edges: impl IntoIterator<Item = (u32, u32)>,
        p: Pos2,
    ) -> Option<(f32, EdgePoint)> {
        edges
            .into_iter()
            .filter_map(|(a, b)| {
                let (x, y) = (self.points[a as usize]?, self.points[b as usize]?);
                let line = y - x;
                let t = ((p - x).dot(line) / line.length_sq().max(0.001)).clamp(0.0, 1.0);
                Some((p.distance(x + line * t), ((a, b), t)))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    fn edge(&self, mesh: &Mesh, face: Option<usize>, p: Pos2) -> Option<EdgePoint> {
        let hit = if let Some(i) = face {
            self.nearest_edge(sides(&mesh.faces[i]), p)
        } else {
            self.nearest_edge(mesh.edges(), p)
        };
        hit.filter(|(dist, _)| *dist < 22.0).map(|(_, edge)| edge)
    }

    fn closest_face_edge(&self, mesh: &Mesh, p: Pos2) -> Option<usize> {
        mesh.faces
            .iter()
            .enumerate()
            .filter_map(|(i, f)| {
                let (d, _) = self.nearest_edge(sides(f), p)?;
                (d < 18.0).then_some((i, d))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    fn vertex(&self, mesh: &Mesh, face: Option<usize>, p: Pos2) -> Option<u32> {
        let candidates = face.map_or_else(
            || (0..mesh.points.len() as u32).collect(),
            |i| mesh.faces[i].clone(),
        );
        candidates
            .into_iter()
            .filter_map(|i| {
                let d = p.distance(self.points[i as usize]?);
                (d < 20.0).then_some((i, d))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
}
