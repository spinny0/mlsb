#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod collider;
mod editors;
mod environment;
mod fluid;
mod input;
mod layout;
mod mesh;
mod mesh_editor;
mod scene;
mod surface;
mod viewport;
mod worker;

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use eframe::{
    egui::{self, Color32, Key, PointerButton, RichText, Stroke, Vec2},
    egui_wgpu,
};
use glam::{Quat, Vec3};

use fluid::{DT, QUALITIES};
use input::{Operation, Transform};
use scene::{Kind, Liquid, MAX_OBJECTS, Object, Preset, Scene};
use viewport::{Camera, Display, Renderer, ViewSettings};
use worker::{Job, Snapshot, Worker};

const PANEL: Color32 = Color32::from_rgb(44, 44, 44);
const BAR: Color32 = Color32::from_rgb(33, 33, 33);
const TEXT: Color32 = Color32::from_rgb(226, 226, 226);
const MUTED: Color32 = Color32::from_rgb(170, 170, 170);
const ACCENT: Color32 = Color32::from_rgb(255, 150, 49);
const LINE: Color32 = Color32::from_rgb(66, 66, 66);
const BLUE: Color32 = Color32::from_rgb(57, 102, 163);
const CACHE_LIMIT: usize = 144;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Workspace {
    Layout,
    Simulation,
    Lighting,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Properties {
    Object,
    Material,
    Simulation,
    Render,
}

struct App {
    scene: Scene,
    selected: Option<u32>,
    object_serial: u32,
    camera: Camera,
    mode: Display,
    render_quality: usize,
    sim_quality: usize,
    renderer: Renderer,
    logo: egui::TextureHandle,
    worker: Worker,
    snapshot: Snapshot,
    surface_revision: u64,
    cache: VecDeque<Snapshot>,
    scrub: Option<usize>,
    workspace: Workspace,
    properties: Properties,
    toolbar: bool,
    sidebar: bool,
    shading: bool,
    maximized: bool,
    tool: Option<Operation>,
    snapping: bool,
    gizmo_drag: bool,
    camera_properties: bool,
    epoch: u64,
    busy: bool,
    playing: bool,
    step_once: bool,
    sim_time: f32,
    speed: f32,
    last: Instant,
    last_render: Instant,
    dirty: bool,
    transform: Option<Transform>,
    edit: Option<mesh_editor::Edit>,
    undo: Vec<Scene>,
    redo: Vec<Scene>,
    add: bool,
    file: bool,
    path: String,
    notice: String,
    help: bool,
    rock: Option<(u32, Quat, f32)>,
    inspector_edit: Option<Scene>,
    ruler: bool,
    live_tool: Option<Operation>,
    live_drag: Option<layout::LiveDrag>,
    live_origins: std::collections::BTreeMap<u32, (Vec3, Quat, Option<scene::ColliderMotion>)>,
    cursor: Vec3,
    cursor_placed: bool,
    cross_section: Option<usize>,
    section_offset: f32,
    section_reversed: bool,
    auto_render: bool,
    adaptive_quality: usize,
    search: bool,
    query: String,
    search_cursor: usize,
    redo_transform: Option<Transform>,
    adjust_last: bool,
    area: layout::ViewportArea,
    split_view: bool,
    split_fraction: f32,
    split_editor: layout::ViewportArea,
    outliner_filter: String,
}

impl App {
    fn new(
        ctx: &egui::Context,
        rs: egui_wgpu::RenderState,
        logo: &image::RgbaImage,
    ) -> std::io::Result<Self> {
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = Vec2::new(5.0, 4.0);
        style.spacing.button_padding = Vec2::new(7.0, 3.0);
        style.spacing.slider_width = 136.0;
        style.visuals = egui::Visuals::dark();
        style.visuals.override_text_color = Some(TEXT);
        style.visuals.panel_fill = PANEL;
        style.visuals.window_fill = PANEL;
        style.visuals.window_stroke = Stroke::new(1.0_f32, LINE);
        style.visuals.selection.bg_fill = BLUE;
        style.visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
        style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(55, 55, 55);
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(55, 55, 55);
        style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(74, 74, 74);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(74, 74, 74);
        style.visuals.widgets.active.bg_fill = BLUE;
        ctx.set_style(style);
        let logo = ctx.load_texture(
            "Liquid mark",
            egui::ColorImage::from_rgba_unmultiplied(
                [logo.width() as usize, logo.height() as usize],
                logo.as_raw(),
            ),
            egui::TextureOptions::LINEAR,
        );
        let renderer = Renderer::new(rs);
        let now = Instant::now();
        Ok(Self {
            scene: Scene::preset(Preset::Station),
            selected: Some(2),
            object_serial: MAX_OBJECTS as u32,
            camera: Camera::default(),
            mode: Display::Surface,
            render_quality: 1,
            sim_quality: 1,
            renderer,
            logo,
            worker: Worker::start()?,
            snapshot: Snapshot::default(),
            surface_revision: 0,
            cache: VecDeque::new(),
            scrub: None,
            workspace: Workspace::Layout,
            properties: Properties::Object,
            toolbar: true,
            sidebar: false,
            shading: false,
            maximized: false,
            tool: None,
            snapping: false,
            gizmo_drag: false,
            camera_properties: false,
            epoch: 1,
            busy: false,
            playing: false,
            step_once: false,
            sim_time: 0.0,
            speed: 1.0,
            last: now,
            last_render: now - Duration::from_secs(1),
            dirty: true,
            transform: None,
            edit: None,
            undo: Vec::new(),
            redo: Vec::new(),
            add: false,
            file: false,
            path: "liquid-scene.mlsb.json".into(),
            notice: String::new(),
            help: false,
            rock: None,
            inspector_edit: None,
            ruler: false,
            live_tool: None,
            live_drag: None,
            live_origins: std::collections::BTreeMap::new(),
            cursor: Vec3::ZERO,
            cursor_placed: false,
            cross_section: None,
            section_offset: 0.0,
            section_reversed: false,
            auto_render: true,
            adaptive_quality: 2,
            search: false,
            query: String::new(),
            search_cursor: 0,
            redo_transform: None,
            adjust_last: false,
            area: layout::ViewportArea::Scene,
            split_view: false,
            split_fraction: 0.72,
            split_editor: layout::ViewportArea::Physics,
            outliner_filter: String::new(),
        })
    }

    fn remember(&mut self, before: Scene) {
        self.redo_transform = None;
        if self.undo.len() == 40 {
            self.undo.remove(0);
        }
        self.undo.push(before);
        self.redo.clear();
        self.dirty = true;
    }

    fn history(&mut self, redo: bool) {
        self.redo_transform = None;
        let next = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };
        if let Some(next) = next {
            let current = std::mem::replace(&mut self.scene, next);
            if redo {
                self.undo.push(current);
            } else {
                self.redo.push(current);
            }
            self.selected = self
                .selected
                .filter(|id| self.scene.objects.iter().any(|o| o.id == *id));
            if self.selection().is_none_or(|o| o.mesh.is_none()) {
                self.edit = None;
            }
            self.rock = None;
            self.dirty = true;
        }
    }

    fn reset(&mut self) {
        self.live_drag = None;
        for (id, (position, rotation, motion)) in std::mem::take(&mut self.live_origins) {
            if let Some(object) = self.scene.objects.iter_mut().find(|object| object.id == id) {
                object.position = position;
                object.rotation = rotation;
                object.motion = motion;
            }
        }
        if let Some((id, base, _)) = self.rock.take()
            && let Some(tank) = self.scene.objects.iter_mut().find(|o| o.id == id)
        {
            tank.rotation = base;
        }
        self.epoch = self.epoch.wrapping_add(1);
        self.sim_time = 0.0;
        self.step_once = false;
        self.snapshot = Snapshot::default();
        self.cache.clear();
        self.scrub = None;
        self.surface_revision = self.surface_revision.wrapping_add(1);
        self.dirty = true;
    }

    fn set_preset(&mut self, preset: Preset) {
        self.live_origins.clear();
        self.scene = Scene::preset(preset);
        self.selected = if preset == Preset::Station {
            Some(2)
        } else {
            self.scene.objects.first().map(|o| o.id)
        };
        self.camera = Camera::default();
        self.camera_properties = false;
        self.playing = false;
        self.rock = None;
        self.transform = None;
        self.live_tool = None;
        self.redo_transform = None;
        self.cursor = Vec3::ZERO;
        self.cursor_placed = false;
        self.edit = None;
        self.undo.clear();
        self.redo.clear();
        self.notice.clear();
        self.reset();
    }

    fn add_object(&mut self, kind: Kind) {
        if self.scene.objects.len() >= MAX_OBJECTS {
            self.notice = "Scene limit reached (16 objects).".into();
            return;
        }
        if self.edit.is_some() {
            self.toggle_edit();
        }
        self.remember(self.scene.clone());
        let id = self.next_object_id();
        let y = match kind {
            Kind::Emitter => 1.95,
            Kind::Drain => 0.24,
            Kind::Box | Kind::Tank => 0.60,
            Kind::Light => 2.25,
            Kind::Camera => 1.45,
            Kind::Field => 0.55,
        };
        let pos = if self.cursor_placed {
            self.cursor
        } else {
            Vec3::new(
                self.camera.pivot.x.clamp(-1.3, 1.3),
                y,
                self.camera.pivot.z.clamp(-0.8, 0.8),
            )
        };
        let mut object = Object::new(id, kind, pos);
        if kind == Kind::Light {
            let light = &self.scene.lighting;
            let a = light.azimuth.to_radians();
            let e = light.elevation.to_radians();
            object.rotation = Quat::from_rotation_arc(
                Vec3::Y,
                Vec3::new(a.sin() * e.cos(), e.sin(), a.cos() * e.cos()),
            );
            self.scene.active_light = Some(id);
        }
        if kind == Kind::Camera {
            let (_, right, up, forward) = self.camera.basis();
            object.rotation = Quat::from_mat3(&glam::Mat3::from_cols(right, up, -forward));
        }
        self.scene.objects.push(object);
        self.selected = Some(id);
        self.add = false;
    }

    fn duplicate(&mut self) {
        if self.scene.objects.len() == MAX_OBJECTS {
            return;
        }
        let Some(mut object) = self.selection().cloned() else {
            return;
        };
        let before = self.scene.clone();
        let old_selection = self.selected;
        object.id = self.next_object_id();
        object.name = format!("{} copy", object.name.chars().take(42).collect::<String>());
        self.selected = Some(object.id);
        self.scene.objects.push(object);
        if !self.scene.within_mesh_budget() {
            self.scene = before;
            self.notice = "Scene mesh limit reached.".into();
            self.selected = old_selection;
            return;
        }
        self.transform = Transform::new(&self.scene, self.selected.unwrap(), Operation::Move);
        if let Some(transform) = &mut self.transform {
            transform.before = before;
        }
        self.dirty = true;
    }

    fn delete(&mut self) {
        if let Some(id) = self.selected.take() {
            self.edit = None;
            self.remember(self.scene.clone());
            self.scene.objects.retain(|o| o.id != id);
            if self.scene.gate == Some(id) {
                self.scene.gate = None;
            }
            if self.scene.active_light == Some(id) {
                self.scene.active_light = None;
            }
            if self.rock.is_some_and(|rock| rock.0 == id) {
                self.rock = None;
            }
        }
    }

    fn selection(&self) -> Option<&Object> {
        self.scene
            .objects
            .iter()
            .find(|o| Some(o.id) == self.selected)
    }

    fn next_object_id(&mut self) -> u32 {
        loop {
            self.object_serial = self
                .object_serial
                .wrapping_add(1)
                .max(MAX_OBJECTS as u32 + 1);
            if self
                .scene
                .objects
                .iter()
                .all(|o| o.id != self.object_serial)
            {
                return self.object_serial;
            }
        }
    }

    fn animate(&mut self, dt: f32, visible: bool) {
        if let Ok(frame) = self.worker.frames.try_recv() {
            self.busy = false;
            if frame.epoch == self.epoch {
                if self.scrub.is_none()
                    && self
                        .cache
                        .back()
                        .is_none_or(|last| frame.time - last.time >= 4.0 * DT - 0.001)
                {
                    if self.cache.len() >= CACHE_LIMIT {
                        self.cache.pop_front();
                    }
                    self.cache.push_back(frame.clone());
                }
                self.snapshot = frame;
                self.surface_revision = self.surface_revision.wrapping_add(1);
                self.dirty = true;
            }
        }
        if self.playing && visible {
            self.sim_time = (self.sim_time + dt * self.speed).min(DT * 2.35);
        }
        if self.busy || !visible {
            return;
        }
        let reset = self.snapshot.epoch != self.epoch;
        let steps = if reset {
            0
        } else if self.step_once {
            1
        } else if self.playing {
            (self.sim_time / DT) as u32
        } else {
            0
        };
        if !reset && steps == 0 {
            return;
        }
        if let Some((id, base, t)) = &mut self.rock
            && let Some(tank) = self.scene.objects.iter_mut().find(|o| o.id == *id)
        {
            *t += steps as f32 * DT;
            let phase = (*t / 1.55).min(1.0);
            let tilt = (phase * std::f32::consts::TAU).sin() * 0.25;
            tank.rotation = *base * Quat::from_rotation_z(tilt);
            if phase >= 1.0 {
                tank.rotation = *base;
                self.rock = None;
            }
            self.dirty = true;
        }
        let job = Job {
            epoch: self.epoch,
            scene: self.scene.clone(),
            quality: QUALITIES[self.sim_quality],
            steps,
        };
        match self.worker.jobs.try_send(job) {
            Ok(()) => {
                self.busy = true;
                self.sim_time = (self.sim_time - steps as f32 * DT).max(0.0);
                if steps > 0 {
                    self.step_once = false;
                }
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                self.playing = false;
                self.notice = "The simulation worker stopped.".into();
            }
            Err(std::sync::mpsc::TrySendError::Full(_)) => {}
        }
    }

    fn show_frame(&mut self, frame: Option<usize>) {
        if self.scrub == frame {
            return;
        }
        self.scrub = frame;
        if frame.is_some() {
            self.playing = false;
            self.step_once = false;
            self.sim_time = 0.0;
        }
        self.surface_revision = self.surface_revision.wrapping_add(1);
        self.dirty = true;
    }

    fn playback(&mut self) {
        if self.edit.is_some() {
            self.toggle_edit();
        }
        self.show_frame(None);
        self.playing = !self.playing;
        self.adaptive_quality = self.render_quality;
    }

    fn header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("brand")
            .exact_height(29.0)
            .frame(panel(Color32::from_gray(25), 4))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.image((self.logo.id(), Vec2::splat(17.0)))
                        .on_hover_text("MLSB 0.0.7");
                    ui.menu_button("File", |ui| {
                        if ui.button("New Scene").clicked() {
                            self.set_preset(Preset::Empty);
                            ui.close();
                        }
                        if ui.button("Open / Save Scene…").clicked() {
                            self.file = true;
                            ui.close();
                        }
                    });
                    ui.menu_button("Edit", |ui| {
                        if ui.button("Undo    Ctrl+Z").clicked() {
                            self.history(false);
                            ui.close();
                        }
                        if ui.button("Redo    Ctrl+Shift+Z").clicked() {
                            self.history(true);
                            ui.close();
                        }
                        if ui.button("Operator Search    F3").clicked() {
                            self.open_search();
                            ui.close();
                        }
                    });
                    ui.menu_button("Render", |ui| {
                        self.shading_menu(ui);
                    });
                    ui.menu_button("Window", |ui| {
                        self.area_actions(ui);
                    });
                    ui.menu_button("Help", |ui| {
                        if ui.button("Controls").clicked() {
                            self.help = true;
                            ui.close();
                        }
                    });
                    ui.separator();
                    for (workspace, label, properties) in [
                        (Workspace::Layout, "Layout", Properties::Object),
                        (Workspace::Simulation, "Physics", Properties::Simulation),
                        (Workspace::Lighting, "Shading", Properties::Render),
                    ] {
                        if ui
                            .selectable_label(self.workspace == workspace, label)
                            .clicked()
                        {
                            self.workspace = workspace;
                            self.properties = properties;
                            self.camera_properties = false;
                        }
                    }
                    ui.separator();
                    self.preset_menu(ui, "active scene");
                });
            });
    }

    fn timeline(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("playback")
            .default_height(70.0)
            .height_range(57.0..=180.0)
            .resizable(true)
            .frame(panel(BAR, 5))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Timeline");
                    ui.separator();
                    let title = if self.playing { "Ⅱ" } else { "▶" };
                    if ui
                        .add(egui::Button::new(RichText::new(title).strong()).fill(BLUE))
                        .on_hover_text("Space")
                        .clicked()
                    {
                        self.playback();
                    }
                    if ui.button("Step").on_hover_text("Right arrow").clicked() {
                        if self.edit.is_some() {
                            self.toggle_edit();
                        }
                        self.show_frame(None);
                        self.playing = false;
                        self.step_once = true;
                    }
                    if ui
                        .button("Reset")
                        .on_hover_text("Restore the seeded liquid state")
                        .clicked()
                    {
                        self.reset();
                    }
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "Frame {}",
                            (self
                                .scrub
                                .and_then(|i| self.cache.get(i))
                                .map_or(self.snapshot.time, |f| f.time)
                                / DT)
                                .round() as u32
                        ))
                        .monospace()
                        .color(TEXT),
                    );
                    egui::ComboBox::from_id_salt("speed")
                        .selected_text(format!("{}×", self.speed))
                        .width(54.0)
                        .show_ui(ui, |ui| {
                            for speed in [0.25, 0.5, 1.0] {
                                ui.selectable_value(&mut self.speed, speed, format!("{speed}×"));
                            }
                        });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let n = self.snapshot.particles.len();
                        let limit = QUALITIES[self.sim_quality].capacity;
                        let saturated = n >= limit;
                        ui.label(RichText::new(format!("{n} / {limit} particles")).color(
                            if saturated {
                                Color32::from_rgb(250, 197, 117)
                            } else {
                                MUTED
                            },
                        ));
                        ui.separator();
                        ui.label(
                            RichText::new(format!(
                                "{:.1} ms solve",
                                self.snapshot.step_time.as_secs_f32() * 1000.0
                            ))
                            .color(MUTED),
                        )
                        .on_hover_text(
                            "CPU time per physics step. The viewport renders on the GPU.",
                        );
                        if self.playing && self.snapshot.step_time.as_secs_f32() > DT {
                            ui.label(
                                RichText::new("Simulation slowing")
                                    .color(Color32::from_rgb(250, 197, 117)),
                            );
                        }
                    });
                });
                ui.horizontal(|ui| {
                    let total = self.cache.len();
                    if ui.selectable_label(self.scrub.is_none(), "Live").clicked() {
                        self.show_frame(None);
                    }
                    let mut selected = self.scrub.unwrap_or(total);
                    ui.label(RichText::new("Recent frames").color(MUTED));
                    ui.spacing_mut().slider_width = (ui.available_width() - 83.0).max(45.0);
                    if ui
                        .add_enabled(
                            total > 0,
                            egui::Slider::new(&mut selected, 0..=total).show_value(false),
                        )
                        .on_hover_text("Scrub recorded fluid and collider states")
                        .changed()
                    {
                        self.show_frame((selected < total).then_some(selected));
                    }
                    ui.label(if self.scrub.is_some() {
                        "Paused"
                    } else if self.playing {
                        "Playing"
                    } else {
                        "Ready"
                    });
                });
            });
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if !self.add
            && !self.file
            && !self.help
            && !self.search
            && !self.adjust_last
            && self.live_drag.is_none()
            && self.transform.is_none()
        {
            if ctx.input(|i| i.key_pressed(Key::F3)) {
                self.open_search();
                return;
            }
            if ctx.input(|i| i.key_pressed(Key::F9)) && self.redo_transform.is_some() {
                self.adjust_last = true;
                return;
            }
        }
        if !self.add && !self.file && !self.help && self.transform.is_none() {
            let focused = ctx.memory(|m| m.focused());
            let editing_text =
                focused.is_some_and(|id| egui::TextEdit::load_state(ctx, id).is_some());
            if !editing_text
                && !self.search
                && !self.adjust_last
                && self.live_drag.is_none()
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Tab))
            {
                ctx.memory_mut(|m| {
                    m.stop_text_input();
                    m.move_focus(egui::FocusDirection::None);
                });
                self.toggle_edit();
                return;
            }
        }
        if ctx.wants_keyboard_input()
            || self.add
            || self.file
            || self.help
            || self.search
            || self.adjust_last
            || self.live_drag.is_some()
            || self.transform.is_some()
        {
            return;
        }
        let (shift, ctrl) = ctx.input(|i| (i.modifiers.shift, i.modifiers.command));
        let pressed = |key| ctx.input(|i| i.key_pressed(key));
        if self.edit.is_some() {
            if ctrl && pressed(Key::Space) {
                self.maximized = !self.maximized;
                self.dirty = true;
            } else if !ctrl && pressed(Key::Home) {
                self.frame_scene();
            } else if shift && pressed(Key::A) {
                self.toggle_edit();
                self.add = true;
            }
            self.edit_keys(ctx);
            return;
        }
        if ctrl && pressed(Key::Space) {
            self.maximized = !self.maximized;
            self.dirty = true;
        } else if pressed(Key::Space) {
            self.playback();
        }
        if pressed(Key::ArrowRight) {
            self.show_frame(None);
            self.playing = false;
            self.step_once = true;
        }
        if !ctrl && !shift {
            if pressed(Key::T) {
                self.toolbar = !self.toolbar;
            }
            if pressed(Key::N) {
                self.sidebar = !self.sidebar;
            }
            if pressed(Key::Z) {
                self.shading = !self.shading;
            }
            if pressed(Key::Escape) {
                self.shading = false;
            }
        }
        if ctrl && pressed(Key::Z) {
            self.history(shift);
        }
        if ctrl && pressed(Key::S) {
            self.file = true;
        }
        if shift && pressed(Key::A) {
            self.add = true;
        }
        if shift && pressed(Key::D) {
            self.duplicate();
        }
        if pressed(Key::Delete) || (!ctrl && pressed(Key::X)) {
            self.delete();
        }
        if pressed(Key::Home) {
            self.frame_scene();
        }
        if pressed(Key::Period)
            && let Some(o) = self.selection()
        {
            let (p, h) = (o.position, o.half.max_element());
            self.camera.pivot = p;
            self.camera.distance = (h * 4.8).clamp(0.9, 8.0);
            self.dirty = true;
        }
        if pressed(Key::Num5) {
            self.camera.ortho = !self.camera.ortho;
            self.dirty = true;
        }
        for (key, yaw, pitch) in [
            (Key::Num1, 0.0, 0.0),
            (Key::Num3, std::f32::consts::FRAC_PI_2, 0.0),
            (Key::Num7, 0.0, std::f32::consts::FRAC_PI_2),
        ] {
            if pressed(key) {
                self.camera.yaw = yaw + if ctrl { std::f32::consts::PI } else { 0.0 };
                self.camera.pitch = if ctrl { -pitch } else { pitch };
                self.camera.ortho = true;
                self.dirty = true;
            }
        }
        if !ctrl
            && !shift
            && let Some(id) = self.selected
        {
            for (key, op) in [
                (Key::G, Operation::Move),
                (Key::R, Operation::Rotate),
                (Key::S, Operation::Scale),
            ] {
                if pressed(key) {
                    self.transform = Transform::new(&self.scene, id, op);
                }
            }
        }
    }

    fn viewport(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(panel(BAR, 0))
            .show(ctx, |ui| {
                if !self.maximized {
                    self.viewport_header(ui);
                }
                if self.area != layout::ViewportArea::Scene {
                    self.alternate_editor(ui);
                    return;
                }
                let available = ui.available_size();
                let width = if self.split_view {
                    available.x * self.split_fraction
                } else {
                    available.x
                };
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(width, available.y).max(Vec2::splat(8.0)),
                    egui::Sense::click_and_drag(),
                );
                if self.edit.is_some() {
                    self.edit_pointer(ctx, rect, &response);
                }
                let pointer_owned = self.live_pointer(ctx, rect, &response);
                if !pointer_owned
                    && self.edit.is_none()
                    && self.transform.is_none()
                    && response.hovered()
                    && let Some(pointer) = ctx.input(|i| {
                        (i.pointer.primary_pressed() && !i.modifiers.alt)
                            .then(|| i.pointer.interact_pos())
                            .flatten()
                    })
                {
                    self.start_gizmo(rect, pointer);
                }
                if let Some(transform) = &mut self.transform {
                    input::read_transform(transform, ctx);
                    transform.snap = self.snapping || ctx.input(|i| i.modifiers.ctrl);
                    if response.hovered() || self.gizmo_drag {
                        transform.delta += ctx.input(|i| i.pointer.delta());
                    }
                    if let Some(object) = self
                        .scene
                        .objects
                        .iter_mut()
                        .find(|o| o.id == transform.original.id)
                    {
                        transform.apply(object, self.camera, rect.height());
                        self.dirty = true;
                    }
                    if ctx.input(|i| i.key_pressed(Key::Escape))
                        || response.clicked_by(PointerButton::Secondary)
                    {
                        self.scene = self.transform.take().unwrap().before;
                        self.gizmo_drag = false;
                        self.selected = self
                            .selected
                            .filter(|id| self.scene.objects.iter().any(|o| o.id == *id));
                    } else if ctx.input(|i| i.key_pressed(Key::Enter))
                        || response.clicked_by(PointerButton::Primary)
                        || (self.gizmo_drag && ctx.input(|i| i.pointer.primary_released()))
                    {
                        let transform = self.transform.take().unwrap();
                        let before = transform.before.clone();
                        self.gizmo_drag = false;
                        self.remember(before);
                        self.redo_transform = Some(transform);
                    }
                } else if !pointer_owned {
                    let mods = ctx.input(|i| i.modifiers);
                    let orbit = response.dragged_by(PointerButton::Middle)
                        || (mods.alt && response.dragged_by(PointerButton::Primary));
                    if orbit {
                        let delta = ctx.input(|i| i.pointer.delta());
                        if mods.shift {
                            let (_, right, up, _) = self.camera.basis();
                            self.camera.pivot +=
                                (up * delta.y - right * delta.x) * self.camera.distance * 0.75
                                    / rect.height().max(1.0);
                        } else if mods.ctrl {
                            self.zoom(-delta.y * 4.0);
                        } else {
                            self.camera.yaw -= delta.x * 0.008;
                            self.camera.pitch =
                                (self.camera.pitch + delta.y * 0.008).clamp(-1.55, 1.55);
                        }
                        self.dirty = true;
                    }
                    if response.hovered() {
                        let scroll = ctx.input(|i| i.raw_scroll_delta.y);
                        if scroll != 0.0 && !self.edit_scroll_owned() {
                            self.zoom(scroll);
                        }
                    }
                    if self.edit.is_none()
                        && response.clicked_by(PointerButton::Primary)
                        && !mods.alt
                        && let Some(pointer) = response.interact_pointer_pos()
                    {
                        let tick = (self.snapshot.time / DT).round() as u32;
                        self.selected =
                            input::pick(&self.scene.at_frame(tick), self.camera, rect, pointer);
                        self.camera_properties = false;
                        self.dirty = true;
                    }
                }
                let detail = if self.auto_render && self.playing {
                    self.adaptive_quality.min(self.render_quality)
                } else {
                    self.render_quality
                };
                let scale = ctx.pixels_per_point() * [0.57, 0.86, 1.0][detail];
                let size = rect.size() * scale;
                let cap = if self.mode == Display::RayTraced {
                    Vec2::new(1120.0, 800.0)
                } else {
                    Vec2::new(1440.0, 1000.0)
                };
                let clamp = (cap.x / size.x).min(cap.y / size.y).min(1.0);
                let size = size * clamp;
                let dimensions = [
                    size.x.round().max(16.0) as u32,
                    size.y.round().max(16.0) as u32,
                ];
                if self.renderer.ready() && self.last_render.elapsed() >= Duration::from_millis(15)
                {
                    self.dirty |= self.renderer.resize(dimensions);
                    if self.dirty {
                        let frame = self
                            .scrub
                            .and_then(|i| self.cache.get(i))
                            .unwrap_or(&self.snapshot);
                        let cached_scene = self.scrub.and(frame.scene.as_ref()).map(|recorded| {
                            let mut scene = recorded.clone();
                            scene.lighting = self.scene.lighting.clone();
                            scene.active_light = self.scene.active_light;
                            scene.liquid = self.scene.liquid;
                            scene
                        });
                        let source = cached_scene.as_ref().unwrap_or(&self.scene);
                        let effective = if self.edit.is_some() {
                            std::borrow::Cow::Borrowed(source)
                        } else {
                            source.at_frame((frame.time / DT).round() as u32)
                        };
                        self.renderer.clip_plane = self.cross_section.map(|axis| {
                            let sign = if self.section_reversed { -1.0 } else { 1.0 };
                            (input::AXES[axis] * sign, self.section_offset * sign)
                        });
                        self.renderer.selection_outline = self.edit.is_none();
                        self.renderer.render_live(
                            &effective,
                            &frame.particles,
                            &frame.secondary,
                            ViewSettings {
                                camera: self.camera,
                                selected: self.selected,
                                mode: self.mode,
                                detail,
                                surface_revision: self.surface_revision,
                            },
                        );
                        self.last_render = Instant::now();
                        if self.playing && self.auto_render && self.renderer.last_frame_ms() > 27 {
                            self.adaptive_quality = self.adaptive_quality.saturating_sub(1);
                        }
                        self.dirty = false;
                    }
                }
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 0.0, BAR);
                if let Some(id) = self.renderer.texture_id() {
                    painter.image(
                        id,
                        rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                self.overlay(rect, &painter);
                self.scene_markers(rect, &painter);
                self.layout_overlays(ui, rect, &painter);
                if self.edit.is_some() {
                    self.mesh_overlay(rect, &painter);
                } else {
                    self.paint_gizmo(rect, &painter);
                }
                self.viewport_tools(ui, rect);
                if self.split_view {
                    self.show_view_split(ui, rect);
                }
            });
    }

    fn zoom(&mut self, scroll: f32) {
        self.camera.distance = (self.camera.distance * (-scroll * 0.0023).exp()).clamp(0.52, 18.0);
        self.dirty = true;
    }

    fn scene_markers(&self, rect: egui::Rect, painter: &egui::Painter) {
        for object in self.scene.objects.iter().filter(|o| {
            o.visible && o.enabled && matches!(o.kind, Kind::Camera | Kind::Light | Kind::Field)
        }) {
            let Some(position) = input::project(self.camera, rect, object.position) else {
                continue;
            };
            let direction = object.rotation
                * match object.kind {
                    Kind::Light => -Vec3::Y,
                    Kind::Field => Vec3::Y,
                    _ => Vec3::NEG_Z,
                };
            let color = if self.selected == Some(object.id) {
                ACCENT
            } else {
                MUTED
            };
            if let Some(tip) = input::project(self.camera, rect, object.position + direction * 0.30)
            {
                painter.arrow(position, tip - position, Stroke::new(1.1_f32, color));
            }
            painter.circle_stroke(position, 4.0, Stroke::new(1.5_f32, color));
            painter.text(
                position + egui::vec2(7.0, -5.0),
                egui::Align2::LEFT_BOTTOM,
                &object.name,
                egui::FontId::proportional(11.0),
                color,
            );
        }
    }

    fn frame_scene(&mut self) {
        let mut min = Vec3::new(-1.6, 0.0, -0.92);
        let mut max = Vec3::new(1.6, 1.65, 0.92);
        for object in self
            .scene
            .objects
            .iter()
            .filter(|o| o.enabled && !matches!(o.kind, Kind::Light | Kind::Camera))
        {
            let m = glam::Mat3::from_quat(object.rotation);
            let half = m.x_axis.abs() * object.half.x
                + m.y_axis.abs() * object.half.y
                + m.z_axis.abs() * object.half.z;
            min = min.min(object.position - half);
            max = max.max(object.position + half);
        }
        self.camera.pivot = (min + max) * 0.5;
        self.camera.distance = ((max - min).length() * 1.31).clamp(2.0, 16.0);
        self.dirty = true;
    }

    fn overlay(&self, rect: egui::Rect, painter: &egui::Painter) {
        let label = self.transform.as_ref().map_or_else(
            || {
                format!(
                    "User {}\n{}",
                    if self.camera.ortho {
                        "Orthographic"
                    } else {
                        "Perspective"
                    },
                    self.selection().map_or("", |object| object.name.as_str()),
                )
            },
            Transform::label,
        );
        let font = egui::FontId::proportional(12.0);
        let text = painter.layout_no_wrap(
            label,
            font,
            if self.transform.is_some() {
                ACCENT
            } else {
                TEXT
            },
        );
        let pos = rect.min + Vec2::new(54.0, 18.0);
        painter.galley(pos, text, TEXT);
        if let Some(o) = self
            .selection()
            .filter(|o| o.kind == Kind::Emitter && o.enabled)
        {
            let forward = o.rotation * Vec3::NEG_Y;
            let p = o.position + forward * 0.09;
            if let (Some(a), Some(b)) = (
                input::project(self.camera, rect, p),
                input::project(self.camera, rect, p + forward * 0.38),
            ) {
                painter.arrow(a, b - a, Stroke::new(1.6_f32, ACCENT));
                if let (Some(l), Some(r)) = (
                    input::project(self.camera, rect, p - o.rotation * Vec3::X * o.radius),
                    input::project(self.camera, rect, p + o.rotation * Vec3::X * o.radius),
                ) {
                    painter.line_segment([l, r], Stroke::new(1.3_f32, ACCENT));
                }
            }
        }
        if self.ruler
            && let Some(tank) = self
                .selection()
                .filter(|o| o.kind == Kind::Tank && o.enabled)
        {
            let local = Vec3::new(tank.half.x + 0.025, -tank.half.y + scene::WALL, tank.half.z);
            let at = |height| {
                input::project(
                    self.camera,
                    rect,
                    tank.position + tank.rotation * (local + Vec3::Y * height),
                )
            };
            let max = tank.half.y * 2.0 - scene::WALL;
            if let (Some(a), Some(b)) = (at(0.0), at(max)) {
                painter.line_segment([a, b], Stroke::new(1.2_f32, ACCENT));
                for step in 0..=(max * 4.0) as u32 {
                    let height = step as f32 * 0.25;
                    if let Some(p) = at(height) {
                        painter.line_segment(
                            [p, p + Vec2::new(7.0, 0.0)],
                            Stroke::new(1.0_f32, ACCENT),
                        );
                        painter.text(
                            p + Vec2::new(10.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            format!("{height:.2} m"),
                            egui::FontId::monospace(10.0),
                            TEXT,
                        );
                    }
                }
            }
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.add {
            let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
            egui::Window::new("Add object")
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    section(ui, "Mesh");
                    ui.horizontal_wrapped(|ui| {
                        ui.set_max_width(320.0);
                        for primitive in mesh::Primitive::ALL {
                            if ui.button(primitive.name()).clicked() {
                                self.add_primitive(primitive);
                            }
                        }
                    });
                    ui.separator();
                    section(ui, "Simulation");
                    ui.horizontal_wrapped(|ui| {
                        for kind in [Kind::Emitter, Kind::Drain, Kind::Tank, Kind::Field] {
                            if ui.button(kind.label()).clicked() {
                                self.add_object(kind);
                            }
                        }
                    });
                    ui.separator();
                    section(ui, "Scene");
                    ui.horizontal(|ui| {
                        for kind in [Kind::Light, Kind::Camera] {
                            if ui.button(kind.label()).clicked() {
                                self.add_object(kind);
                            }
                        }
                    });
                    close |= ui.button("Cancel").clicked();
                });
            if close {
                self.add = false;
            }
        }
        if self.file {
            let mut open = self.file;
            egui::Window::new("Scene file")
                .open(&mut open)
                .collapsible(false)
                .default_width(405.0)
                .show(ctx, |ui| {
                    ui.label("Save or load a scene setup.");
                    ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(f32::INFINITY));
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            let result = serde_json::to_vec_pretty(&self.scene)
                                .map_err(|e| e.to_string())
                                .and_then(|bytes| {
                                    std::fs::write(self.path.trim(), bytes)
                                        .map_err(|e| e.to_string())
                                });
                            self.notice = result.map_or_else(
                                |e| format!("Save failed. {e}"),
                                |()| "Scene saved.".into(),
                            );
                        }
                        if ui.button("Open").clicked() {
                            self.notice = match load_scene(self.path.trim()) {
                                Ok(scene) => {
                                    self.live_origins.clear();
                                    self.scene = scene;
                                    self.selected = None;
                                    self.playing = false;
                                    self.transform = None;
                                    self.edit = None;
                                    self.rock = None;
                                    self.undo.clear();
                                    self.redo.clear();
                                    self.reset();
                                    "Scene opened.".into()
                                }
                                Err(error) => format!("Open failed. {error}"),
                            };
                        }
                    });
                    if !self.notice.is_empty() {
                        ui.label(&self.notice);
                    }
                });
            self.file = open;
        }
        if self.help {
            egui::Window::new("Blender controls")
                .open(&mut self.help)
                .resizable(false)
                .show(ctx, |ui| {
                    for (action, keys) in [
                        ("Orbit / pan / zoom", "MMB / Shift+MMB / Wheel"),
                        ("Orbit without a middle button", "Alt+left drag"),
                        ("Select / frame / frame all", "Click / Numpad . / Home"),
                        ("Front / right / top / projection", "1 / 3 / 7 / 5"),
                        ("Move / rotate / scale", "G / R / S"),
                        ("Object / Edit Mode", "Tab"),
                        ("Vertex / edge / face", "1 / 2 / 3 in Edit Mode"),
                        ("Extrude / fill / inset / knife", "E / F / I / K"),
                        ("Loop cut / bevel edge", "Ctrl+R / Ctrl+B"),
                        ("Edge slide", "G twice on an edge or vertex"),
                        ("Tools / transforms / shading", "T / N / Z"),
                        ("Maximize viewport", "Ctrl+Space"),
                        ("Snap during a transform", "Ctrl"),
                        ("Constrain axis / toggle local", "X Y Z / same axis again"),
                        ("Numeric transform", "R X 30 Enter"),
                        (
                            "Confirm / cancel transform",
                            "Enter or click / Esc or right click",
                        ),
                        ("Add / duplicate / delete", "Shift+A / Shift+D / Delete"),
                        ("Undo / redo scene edits", "Ctrl+Z / Ctrl+Shift+Z"),
                        ("Play / single step", "Space / Right arrow"),
                    ] {
                        ui.horizontal(|ui| {
                            ui.label(action);
                            ui.label(RichText::new(keys).monospace().color(ACCENT));
                        });
                    }
                });
        }
    }
}

fn load_scene(path: &str) -> Result<Scene, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1_048_576 {
        return Err("Scene exceeds 1 MiB.".into());
    }
    let mut scene: Scene = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    scene.validate()?;
    Ok(scene)
}

fn panel(fill: Color32, margin: i8) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .inner_margin(egui::Margin::same(margin))
}

fn inspector_dock(ctx: &egui::Context) -> egui::SidePanel {
    let width = ctx.available_rect().width();
    let viewport_width = (width * 0.60).min(360.0);
    let max = (width - viewport_width).min(400.0);
    egui::SidePanel::right("inspector")
        .default_width(260.0)
        .width_range(224.0_f32.min(max)..=max)
}

fn show_dock(
    ctx: &egui::Context,
    dock: egui::SidePanel,
    margin: i8,
    contents: impl FnOnce(&mut egui::Ui),
) {
    dock.frame(panel(PANEL, margin)).show(ctx, |ui| {
        let (bounds, _) =
            ui.allocate_exact_size(ui.available_size_before_wrap(), egui::Sense::hover());
        // SidePanel remembers its measured width. Keep content that overflows
        // in a child UI so it cannot change the user's dock width.
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bounds));
        child.set_clip_rect(child.clip_rect().intersect(bounds));
        contents(&mut child);
    });
}

fn section(ui: &mut egui::Ui, name: &str) {
    ui.label(RichText::new(name).size(15.0).strong());
}

fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    digits: usize,
) -> bool {
    ui.horizontal(|ui| {
        ui.add_sized(
            [75.0, 18.0],
            egui::Label::new(RichText::new(label).color(MUTED)).halign(egui::Align::LEFT),
        );
        ui.spacing_mut().slider_width = (ui.available_width() - 58.0).max(40.0);
        ui.add(
            egui::Slider::new(value, range)
                .fixed_decimals(digits)
                .clamping(egui::SliderClamping::Edits),
        )
        .changed()
    })
    .inner
}

fn quality_menu(ui: &mut egui::Ui, id: &str, name: &str, selected: &mut usize) {
    let tiers = ["Low", "Balanced", "High"];
    ui.horizontal(|ui| {
        ui.label(RichText::new(name).color(MUTED));
        egui::ComboBox::from_id_salt(id)
            .selected_text(tiers[*selected])
            .width(90.0)
            .show_ui(ui, |ui| {
                for (i, name) in tiers.into_iter().enumerate() {
                    ui.selectable_value(selected, i, name);
                }
            });
    });
}

impl App {
    fn frame(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.06);
        self.last = now;
        let visible = ctx.input(|i| !i.viewport().minimized.unwrap_or(false));
        if visible {
            self.dirty |= self.renderer.poll_environment(&self.scene.lighting);
        }
        self.keys(ctx);
        self.animate(dt, visible);
        if !self.maximized {
            self.header(ctx);
            self.inspector(ctx);
            self.timeline(ctx);
        }
        self.viewport(ctx);
        self.dialogs(ctx);
        self.command_dialogs(ctx);
        if visible {
            if self.playing
                || self.busy
                || self.dirty
                || self.step_once
                || self.transform.is_some()
                || self.renderer.panorama_loading()
            {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        } else {
            ctx.request_repaint_after(Duration::from_millis(180));
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame(ctx);
    }
}

fn main() -> eframe::Result {
    let logo = image::load_from_memory(include_bytes!("../../logo.png"))
        .expect("Liquid logo")
        .into_rgba8();
    let icon = egui::IconData {
        width: logo.width(),
        height: logo.height(),
        rgba: logo.as_raw().clone(),
    };
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_icon(icon)
            .with_inner_size([1380.0, 860.0])
            .with_min_inner_size([950.0, 580.0]),
        wgpu_options: egui_wgpu::WgpuConfiguration {
            present_mode: wgpu::PresentMode::AutoVsync,
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "MLSB 0.0.7 — Making Liquid Sims Better",
        options,
        Box::new(|cc| {
            let rs = cc
                .wgpu_render_state
                .clone()
                .ok_or("The Liquid viewport requires a graphics adapter")?;
            Ok(Box::new(App::new(&cc.egui_ctx, rs, &logo)?))
        }),
    )
}

#[cfg(test)]
mod tests;
