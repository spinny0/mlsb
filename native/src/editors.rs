use super::*;

impl App {
    pub(super) fn inspector(&mut self, ctx: &egui::Context) {
        let mut before = None;
        show_dock(ctx, inspector_dock(ctx), 0, |ui| {
            egui::TopBottomPanel::top("outliner editor")
                .resizable(true)
                .default_height(166.0)
                .height_range(100.0..=(ui.available_height() * 0.64).max(100.0))
                .frame(panel(BAR, 5))
                .show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Outliner");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.outliner_filter)
                                .hint_text("Search")
                                .desired_width(105.0),
                        );
                        if ui
                            .small_button("+")
                            .on_hover_text("Add object · Shift+A")
                            .clicked()
                        {
                            self.add = true;
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt("outliner rows")
                        .show(ui, |ui| {
                            before = self.outliner(ui);
                        });
                });
            ui.add_space(5.0);
            section(ui, "Properties");
            egui::SidePanel::left("property tabs")
                .exact_width(28.0)
                .resizable(false)
                .frame(panel(BAR, 2))
                .show_inside(ui, |ui| {
                    for (tab, icon, label) in [
                        (Properties::Render, "R", "Render and World"),
                        (Properties::Object, "O", "Object"),
                        (Properties::Simulation, "◎", "Physics"),
                        (Properties::Material, "M", "Material"),
                    ] {
                        if ui
                            .selectable_label(self.properties == tab, icon)
                            .on_hover_text(label)
                            .clicked()
                        {
                            self.properties = tab;
                            self.camera_properties = false;
                        }
                    }
                });
            ui.vertical(|ui| {
                egui::ScrollArea::vertical()
                    .id_salt("property scroll")
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        let old = match self.properties {
                            Properties::Object => {
                                if self.edit.is_some() {
                                    self.mesh_panel(ui);
                                    None
                                } else if self.camera_properties {
                                    self.edit_camera(ui);
                                    None
                                } else {
                                    self.edit_object(ui)
                                }
                            }
                            Properties::Material => self.edit_material(ui),
                            Properties::Simulation => self.fluid_properties(ui),
                            Properties::Render => self.render_controls(ui),
                        };
                        if before.is_none() {
                            before = old;
                        }
                        if !self.notice.is_empty() {
                            ui.label(RichText::new(&self.notice).color(ACCENT).small());
                        }
                    });
            });
        });
        self.remember_inspector(ctx, before);
    }

    fn outliner(&mut self, ui: &mut egui::Ui) -> Option<Scene> {
        let prior = self.scene.clone();
        let mut changed = false;
        let mut select = None;
        let query = self.outliner_filter.to_lowercase();
        egui::CollapsingHeader::new("Scene Collection")
            .default_open(true)
            .show(ui, |ui| {
                for object in self
                    .scene
                    .objects
                    .iter_mut()
                    .filter(|o| o.name.to_lowercase().contains(&query))
                {
                    ui.push_id(object.id, |ui| {
                        ui.horizontal(|ui| {
                            let icon = match object.kind {
                                Kind::Camera => "C",
                                Kind::Light => "L",
                                Kind::Field => "↻",
                                Kind::Emitter => "↓",
                                Kind::Drain => "↑",
                                Kind::Box | Kind::Tank => "▪",
                            };
                            ui.label(RichText::new(icon).color(ACCENT));
                            let color = if object.visible && object.enabled {
                                TEXT
                            } else {
                                MUTED
                            };
                            let selected = self.selected == Some(object.id);
                            if ui
                                .selectable_label(
                                    selected,
                                    RichText::new(&object.name).color(color),
                                )
                                .on_hover_text(object.kind.label())
                                .clicked()
                            {
                                select = Some(object.id);
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button(if object.visible { "●" } else { "○" })
                                        .on_hover_text("Toggle viewport visibility")
                                        .clicked()
                                    {
                                        object.visible = !object.visible;
                                        changed = true;
                                    }
                                    if ui
                                        .small_button(if object.selectable { "+" } else { "–" })
                                        .on_hover_text("Toggle viewport selection")
                                        .clicked()
                                    {
                                        object.selectable = !object.selectable;
                                        changed = true;
                                    }
                                },
                            );
                        });
                    });
                }
            });
        if let Some(id) = select {
            if self.selected != Some(id) && self.edit.is_some() {
                self.toggle_edit();
            }
            self.selected = Some(id);
            self.camera_properties = false;
            self.properties = Properties::Object;
            self.dirty = true;
        }
        if self.scene.objects.is_empty() {
            ui.label(RichText::new("Add an object with Shift+A").color(MUTED));
        }
        changed.then_some(prior)
    }

    pub(super) fn fluid_properties(&mut self, ui: &mut egui::Ui) -> Option<Scene> {
        use crate::scene::{FieldKind, FlowMode, FluidRole};
        let prior = self.scene.clone();
        let mut changed = false;
        ui.label("Physics");
        let selected = self.selected;
        let mut role_change = None;
        let mut reset = false;
        if let Some(object) = self
            .scene
            .objects
            .iter_mut()
            .find(|o| Some(o.id) == selected)
        {
            if object.kind == Kind::Field {
                egui::CollapsingHeader::new("Force Field")
                    .default_open(true)
                    .show(ui, |ui| {
                        egui::ComboBox::from_label("Type")
                            .selected_text(format!("{:?}", object.field.kind))
                            .show_ui(ui, |ui| {
                                for kind in
                                    [FieldKind::Directional, FieldKind::Vortex, FieldKind::Radial]
                                {
                                    changed |= ui
                                        .selectable_value(
                                            &mut object.field.kind,
                                            kind,
                                            format!("{kind:?}"),
                                        )
                                        .changed();
                                }
                            });
                        changed |= ui.checkbox(&mut object.enabled, "Enabled").changed();
                        changed |=
                            slider(ui, "Strength", &mut object.field.strength, -60.0..=60.0, 1);
                        changed |= slider(ui, "Radius · m", &mut object.field.radius, 0.1..=5.0, 2);
                    });
            } else if matches!(
                object.kind,
                Kind::Box | Kind::Tank | Kind::Emitter | Kind::Drain
            ) {
                let role = match object.fluid {
                    FluidRole::Automatic if matches!(object.kind, Kind::Emitter | Kind::Drain) => {
                        FluidRole::Flow
                    }
                    FluidRole::Automatic => FluidRole::Effector,
                    role => role,
                };
                egui::CollapsingHeader::new("Fluid")
                    .default_open(true)
                    .show(ui, |ui| {
                        let mut next = role;
                        egui::ComboBox::from_label("Type")
                            .selected_text(format!("{role:?}"))
                            .show_ui(ui, |ui| {
                                for candidate in [
                                    FluidRole::None,
                                    FluidRole::Domain,
                                    FluidRole::Flow,
                                    FluidRole::Effector,
                                ] {
                                    if matches!(object.kind, Kind::Emitter | Kind::Drain)
                                        && matches!(
                                            candidate,
                                            FluidRole::Domain | FluidRole::Effector
                                        )
                                    {
                                        continue;
                                    }
                                    ui.selectable_value(
                                        &mut next,
                                        candidate,
                                        format!("{candidate:?}"),
                                    );
                                }
                            });
                        if next != role {
                            role_change = Some((object.id, next));
                        }
                        changed |= ui.checkbox(&mut object.enabled, "Enabled").changed();
                        if role == FluidRole::Domain {
                            ui.label("Domain Type   Liquid");
                            ui.label("Resize the domain with Scale.");
                        }
                        if role == FluidRole::Flow {
                            ui.label("Flow Type   Liquid");
                            egui::ComboBox::from_label("Flow Behavior")
                                .selected_text(format!("{:?}", object.flow_mode))
                                .show_ui(ui, |ui| {
                                    for mode in
                                        [FlowMode::Geometry, FlowMode::Inflow, FlowMode::Outflow]
                                    {
                                        changed |= ui
                                            .selectable_value(
                                                &mut object.flow_mode,
                                                mode,
                                                format!("{mode:?}"),
                                            )
                                            .changed();
                                    }
                                });
                            if object.flow_mode != FlowMode::Geometry {
                                changed |=
                                    slider(ui, "Radius · m", &mut object.radius, 0.04..=0.55, 2);
                                if object.flow_mode == FlowMode::Inflow
                                    || object.schedule.limited_drain
                                {
                                    changed |=
                                        slider(ui, "Flow · L/s", &mut object.flow, 0.0..=450.0, 0);
                                }
                            }
                            if object.flow_mode == FlowMode::Inflow {
                                changed |=
                                    slider(ui, "Speed · m/s", &mut object.speed, 0.1..=6.0, 2);
                                changed |= ui.checkbox(&mut object.drip, "Dripping").changed();
                                changed |= ui.checkbox(&mut object.oil, "Oil").changed();
                            } else if object.flow_mode == FlowMode::Outflow {
                                changed |= ui
                                    .checkbox(&mut object.schedule.limited_drain, "Limit flow rate")
                                    .changed();
                            } else if ui.button("Reset to fill geometry").clicked() {
                                reset = true;
                            }
                        }
                        if role == FluidRole::Effector {
                            changed |= slider(ui, "Wetting", &mut object.wetting, 0.0..=1.0, 2);
                            if object.mesh.as_ref().is_some_and(|mesh| !mesh.closed()) {
                                changed |= slider(
                                    ui,
                                    "Surface Thickness",
                                    &mut object.thickness,
                                    0.016..=0.30,
                                    3,
                                );
                            }
                        }
                    });
            }
        }
        if let Some((id, role)) = role_change {
            self.scene.assign_role(id, role);
            changed = true;
        }
        if reset {
            self.playing = false;
            self.reset();
        }
        ui.separator();
        egui::CollapsingHeader::new("Liquid Material")
            .default_open(false)
            .show(ui, |ui| {
                changed |= self.edit_material(ui).is_some();
            });
        egui::CollapsingHeader::new("Secondary Particles")
            .default_open(true)
            .show(ui, |ui| {
                let whitewater = &mut self.scene.whitewater;
                ui.horizontal_wrapped(|ui| {
                    changed |= ui.checkbox(&mut whitewater.spray, "Spray").changed();
                    changed |= ui.checkbox(&mut whitewater.foam, "Foam").changed();
                    changed |= ui.checkbox(&mut whitewater.bubbles, "Bubbles").changed();
                });
                changed |= slider(ui, "Amount", &mut whitewater.amount, 0.0..=3.0, 2);
                changed |= slider(ui, "Lifetime", &mut whitewater.lifetime, 0.25..=3.0, 2);
                ui.label(
                    RichText::new(format!("{} active", self.snapshot.secondary.len()))
                        .small()
                        .color(MUTED),
                );
            });
        self.section_controls(ui);
        let simulation = self.simulation_controls(ui);
        if changed { Some(prior) } else { simulation }
    }

    fn edit_object(&mut self, ui: &mut egui::Ui) -> Option<Scene> {
        let Some(id) = self.selected else {
            ui.label(RichText::new("Select an object to edit it.").color(MUTED));
            return None;
        };
        let prior = self.scene.clone();
        let object = self.scene.objects.iter_mut().find(|o| o.id == id)?;
        if ui.button("Physics Properties").clicked() {
            self.properties = Properties::Simulation;
        }
        ui.label(RichText::new(object.kind.label()).color(ACCENT).small());
        let mut changed = ui.text_edit_singleline(&mut object.name).changed();
        let toggle = if matches!(object.kind, Kind::Light | Kind::Camera) {
            "Enabled"
        } else {
            "Enabled in simulation"
        };
        changed |= ui.checkbox(&mut object.enabled, toggle).changed();
        ui.separator();
        changed |= egui::CollapsingHeader::new("Transform")
            .default_open(true)
            .show(ui, |ui| transform_fields(ui, object))
            .body_returned
            .unwrap_or(false);
        if matches!(object.kind, Kind::Emitter | Kind::Drain) {
            ui.separator();
            changed |= slider(ui, "Radius · m", &mut object.radius, 0.04..=0.55, 3);
            if object.kind == Kind::Emitter {
                changed |= slider(ui, "Flow · L/s", &mut object.flow, 0.0..=450.0, 0);
                changed |= slider(ui, "Speed · m/s", &mut object.speed, 0.1..=6.0, 2);
                changed |= ui
                    .checkbox(&mut object.drip, "Build and release drops")
                    .changed();
                ui.horizontal(|ui| {
                    ui.label("Liquid");
                    changed |= ui
                        .selectable_value(&mut object.oil, false, "Scene liquid")
                        .changed();
                    changed |= ui
                        .selectable_value(&mut object.oil, true, "Floating oil")
                        .changed();
                });
            } else {
                changed |= ui
                    .checkbox(&mut object.schedule.limited_drain, "Limit drain rate")
                    .changed();
                if object.schedule.limited_drain {
                    changed |= slider(ui, "Rate · L/s", &mut object.flow, 0.0..=450.0, 0);
                }
            }
            ui.separator();
            section(ui, "Flow schedule");
            let schedule = &mut object.schedule;
            changed |= frame_input(ui, "Start frame", &mut schedule.start, 0);
            let mut stop = schedule.stop.is_some();
            if ui.checkbox(&mut stop, "Set stop frame").changed() {
                schedule.stop = stop.then(|| schedule.start.saturating_add(90));
                changed = true;
            }
            if let Some(end) = &mut schedule.stop {
                *end = (*end).max(schedule.start);
                changed |= frame_input(ui, "Stop frame", end, schedule.start);
            }
            if object.kind == Kind::Emitter || schedule.limited_drain {
                changed |= slider(ui, "Ramp in · s", &mut schedule.ramp, 0.0..=120.0, 2);
            }
            let mut limited = schedule.liters.is_some();
            if ui.checkbox(&mut limited, "Limit total volume").changed() {
                schedule.liters = limited.then_some(100.0);
                changed = true;
            }
            if let Some(liters) = &mut schedule.liters {
                changed |= volume_input(ui, "Total · L", liters, 30_000.0);
            }
            ui.label(
                RichText::new("90 simulation frames per second")
                    .small()
                    .color(MUTED),
            );
        }
        if object.kind == Kind::Tank {
            ui.checkbox(&mut self.ruler, "Fill-height ruler");
        }
        if matches!(object.kind, Kind::Box | Kind::Tank) {
            ui.separator();
            section(ui, "Surface physics");
            changed |= slider(ui, "Wetting", &mut object.wetting, 0.0..=1.0, 2);
            let mut animate = object.motion.is_some();
            if ui.checkbox(&mut animate, "Animate collider").changed() {
                object.motion = animate.then(crate::scene::ColliderMotion::default);
                changed = true;
            }
            if let Some(m) = &mut object.motion {
                changed |= frame_input(ui, "Move from frame", &mut m.start, 0);
                let minimum = m.start.saturating_add(1);
                m.end = m.end.max(minimum);
                changed |= frame_input(ui, "Move to frame", &mut m.end, minimum);
                changed |= slider(ui, "Travel X · m", &mut m.offset.x, -2.0..=2.0, 2);
                changed |= slider(ui, "Travel Y · m", &mut m.offset.z, -2.0..=2.0, 2);
                changed |= slider(ui, "Travel Z · m", &mut m.offset.y, -2.0..=2.0, 2);
                changed |= slider(ui, "Tilt · °", &mut m.tilt, -180.0..=180.0, 1);
            }
        }
        if object.mesh.is_some() {
            changed |= ui
                .checkbox(&mut object.mesh_glass, "Glass material")
                .changed();
            if object.mesh.as_ref().is_some_and(|mesh| !mesh.closed()) {
                changed |= slider(
                    ui,
                    "Collision thickness · m",
                    &mut object.thickness,
                    0.016..=0.30,
                    3,
                );
            }
        }
        if object.kind == Kind::Camera && ui.button("View from camera").clicked() {
            let offset = object.rotation * Vec3::Z;
            self.camera.yaw = offset.x.atan2(offset.z);
            self.camera.pitch = offset.y.clamp(-0.9999, 0.9999).asin();
            self.camera.pivot = object.position - offset * self.camera.distance;
            self.dirty = true;
        }
        if object.kind == Kind::Light {
            let mut active = self.scene.active_light == Some(id);
            if ui.checkbox(&mut active, "Use as key light").changed() {
                self.scene.active_light = active.then_some(id);
                changed = true;
            }
        }
        if changed {
            object.name = object.name.chars().take(48).collect();
        }
        changed.then_some(prior)
    }

    fn edit_material(&mut self, ui: &mut egui::Ui) -> Option<Scene> {
        let prior = self.scene.clone();
        ui.horizontal_wrapped(|ui| {
            for (name, material) in [
                ("Water", Liquid::WATER),
                ("Oil", Liquid::OIL),
                ("Syrup", Liquid::SYRUP),
                ("Honey", Liquid::HONEY),
            ] {
                ui.selectable_value(&mut self.scene.liquid, material, name);
            }
        });
        let m = &mut self.scene.liquid;
        slider(ui, "Viscosity", &mut m.viscosity, 0.0..=1.0, 3);
        slider(ui, "Cohesion", &mut m.cohesion, 0.0..=1.0, 2);
        slider(ui, "Surface tension", &mut m.surface_tension, 0.0..=1.0, 2);
        slider(ui, "Absorption", &mut m.absorption, 0.0..=8.0, 2);
        slider(ui, "Refraction", &mut m.ior, 1.0..=1.55, 3);
        ui.horizontal(|ui| {
            let mut tint = m.tint;
            if ui.color_edit_button_rgb(&mut tint).changed() {
                m.tint = tint;
            }
            ui.label("Transmission tint");
        });
        (*m != prior.liquid).then_some(prior)
    }

    fn simulation_controls(&mut self, ui: &mut egui::Ui) -> Option<Scene> {
        self.preset_menu(ui, "sim preset");
        quality_menu(ui, "simulation detail", "Physics", &mut self.sim_quality);
        let prior = self.scene.clone();
        let mut changed = slider(ui, "Gravity · m/s²", &mut self.scene.gravity, 0.0..=30.0, 2);
        if let Some(fill) = self.scene.fill {
            ui.separator();
            section(ui, "Initial fill");
            let size = fill.max - fill.min;
            let default = size.x * size.y * size.z * 1000.0;
            let mut custom = self.scene.fill_liters.is_some();
            if ui.checkbox(&mut custom, "Set starting volume").changed() {
                self.scene.fill_liters = custom.then_some(default);
                changed = true;
            }
            if let Some(liters) = &mut self.scene.fill_liters {
                let maximum =
                    (size.x * size.z * (crate::fluid::END.y - 0.04 - fill.min.y) * 1000.0)
                        .clamp(0.0, 30_000.0);
                changed |= volume_input(ui, "Starting volume · L", liters, maximum);
            } else {
                ui.label(format!("{default:.1} L"));
            }
            if ui.button("Apply fill and reset").clicked() {
                self.playing = false;
                self.reset();
            }
        }
        match self.scene.preset {
            Preset::Dam => {
                ui.separator();
                section(ui, "Dam gate");
                if self.scene.gate.is_some() {
                    let mut timed = self.scene.gate_frame.is_some();
                    if ui.checkbox(&mut timed, "Schedule release").changed() {
                        self.scene.gate_frame = timed.then_some(180);
                        changed = true;
                    }
                    if let Some(frame) = &mut self.scene.gate_frame {
                        changed |= frame_input(ui, "Open frame", frame, 0);
                        changed |= ui
                            .horizontal(|ui| {
                                ui.label("Lift duration · frames");
                                ui.add(
                                    egui::DragValue::new(&mut self.scene.gate_lift_frames)
                                        .range(0..=900),
                                )
                                .changed()
                            })
                            .inner;
                    }
                }
                if let Some(i) = self
                    .scene
                    .objects
                    .iter()
                    .position(|o| Some(o.id) == self.scene.gate)
                    && ui
                        .button(if self.scene.objects[i].enabled {
                            "Release gate"
                        } else {
                            "Close gate"
                        })
                        .clicked()
                {
                    self.remember(self.scene.clone());
                    self.scene.objects[i].enabled = !self.scene.objects[i].enabled;
                    self.scene.gate_frame = None;
                    self.show_frame(None);
                    if self.edit.is_some() {
                        self.toggle_edit();
                    }
                    self.playing = true;
                }
            }
            Preset::Slosh => {
                if self.rock.is_none()
                    && let Some(o) = self
                        .scene
                        .objects
                        .iter()
                        .find(|o| o.enabled && o.kind == Kind::Tank)
                    && ui.button("Rock tank").clicked()
                {
                    self.rock = Some((o.id, o.rotation, 0.0));
                    self.show_frame(None);
                    if self.edit.is_some() {
                        self.toggle_edit();
                    }
                    self.playing = true;
                }
            }
            _ => {}
        }
        ui.separator();
        let snapshot = self
            .scrub
            .and_then(|i| self.cache.get(i))
            .unwrap_or(&self.snapshot);
        ui.label(
            RichText::new(format!(
                "{:.1} L in scene · frame {}",
                snapshot.particles.len() as f32 * snapshot.particle_liters,
                (snapshot.time / DT).round() as u32
            ))
            .small()
            .color(ACCENT),
        );
        let oil = snapshot.particles.iter().filter(|p| p[3] < 0.0).count();
        if oil > 0 {
            ui.label(format!(
                "{:.1} L floating oil",
                oil as f32 * snapshot.particle_liters
            ));
        }
        ui.label(
            RichText::new(format!(
                "{:.1} L seeded · {:.1} L emitted · {:.1} L drained",
                snapshot.initial_liters,
                snapshot.emitted as f32 * snapshot.particle_liters,
                snapshot.drained as f32 * snapshot.particle_liters,
            ))
            .small()
            .color(MUTED),
        );
        changed.then_some(prior)
    }

    pub(super) fn render_controls(&mut self, ui: &mut egui::Ui) -> Option<Scene> {
        let prior = self.scene.clone();
        self.shading_menu(ui);
        let old = self.render_quality;
        quality_menu(ui, "render detail", "Detail", &mut self.render_quality);
        self.dirty |= old != self.render_quality;
        if old != self.render_quality {
            self.adaptive_quality = self.render_quality;
        }
        if ui
            .checkbox(&mut self.auto_render, "Adaptive viewport while playing")
            .changed()
        {
            self.adaptive_quality = self.render_quality;
            self.dirty = true;
        }
        ui.separator();
        section(ui, "Studio light");
        let light = &mut self.scene.lighting;
        slider(ui, "Azimuth", &mut light.azimuth, -180.0..=180.0, 0);
        slider(ui, "Elevation", &mut light.elevation, 20.0..=90.0, 0);
        slider(ui, "Power", &mut light.power, 0.0..=3.0, 2);
        ui.checkbox(&mut light.caustics, "Caustics in Ray Traced");
        ui.checkbox(&mut light.checker, "Refraction checkerboard");
        ui.separator();
        section(ui, "Environment");
        use crate::scene::EnvironmentMode;
        ui.horizontal_wrapped(|ui| {
            for (mode, label) in [
                (EnvironmentMode::Studio, "Studio"),
                (EnvironmentMode::Garden, "Garden"),
                (EnvironmentMode::File, "HDR / EXR"),
            ] {
                ui.selectable_value(&mut light.environment, mode, label);
            }
        });
        if light.environment == EnvironmentMode::File {
            ui.add(egui::TextEdit::singleline(&mut light.panorama).hint_text("Path to HDR or EXR"));
        }
        if light.environment != EnvironmentMode::Studio {
            slider(
                ui,
                "Rotation",
                &mut light.environment_rotation,
                0.0..=360.0,
                0,
            );
            slider(
                ui,
                "Exposure",
                &mut light.environment_exposure,
                -6.0..=6.0,
                2,
            );
            ui.checkbox(&mut light.environment_background, "Show background");
            egui::ComboBox::from_id_salt("panorama size")
                .selected_text(format!("{} px", light.environment_width))
                .show_ui(ui, |ui| {
                    for size in [1024, 2048, 4096] {
                        if size <= 2048 || light.environment == EnvironmentMode::File {
                            ui.selectable_value(
                                &mut light.environment_width,
                                size,
                                format!("{size} px"),
                            );
                        }
                    }
                });
            ui.label(
                RichText::new(self.renderer.panorama_status())
                    .small()
                    .color(MUTED),
            );
        }
        (*light != prior.lighting).then_some(prior)
    }

    fn edit_camera(&mut self, ui: &mut egui::Ui) {
        let mut yaw = self.camera.yaw.to_degrees();
        let mut pitch = self.camera.pitch.to_degrees();
        let changed = slider(ui, "Orbit", &mut yaw, -180.0..=180.0, 0)
            | slider(ui, "Elevation", &mut pitch, -90.0..=90.0, 0)
            | slider(ui, "Distance", &mut self.camera.distance, 0.52..=18.0, 2)
            | ui.checkbox(&mut self.camera.ortho, "Orthographic")
                .changed();
        if changed {
            self.camera.yaw = yaw.to_radians();
            self.camera.pitch = pitch.to_radians();
            self.dirty = true;
        }
    }

    pub(super) fn preset_menu(&mut self, ui: &mut egui::Ui, id: &str) {
        egui::ComboBox::from_id_salt(id)
            .selected_text(self.scene.preset.label())
            .width(106.0)
            .show_ui(ui, |ui| {
                for preset in Preset::ALL {
                    if ui
                        .selectable_label(self.scene.preset == preset, preset.label())
                        .clicked()
                    {
                        self.set_preset(preset);
                    }
                }
            });
    }

    pub(super) fn shading_menu(&mut self, ui: &mut egui::Ui) {
        let old = self.mode;
        egui::ComboBox::from_id_salt("shading")
            .selected_text(self.mode.label())
            .show_ui(ui, |ui| {
                for mode in [
                    Display::Solid,
                    Display::Surface,
                    Display::RayTraced,
                    Display::Particles,
                ] {
                    ui.selectable_value(&mut self.mode, mode, mode.label());
                }
            });
        self.dirty |= self.mode != old;
    }

    pub(super) fn viewport_tools(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        if self.toolbar {
            let bounds = egui::Rect::from_min_size(
                rect.min + egui::vec2(9.0, 24.0),
                egui::vec2(35.0, 206.0),
            );
            let mut tools = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("viewport tools")
                    .max_rect(bounds),
            );
            panel(BAR, 2).show(&mut tools, |ui| {
                if self.edit.is_none() {
                    for (operation, icon, label) in [
                        (None, "S", "Select"),
                        (Some(Operation::Move), "+", "Move"),
                        (Some(Operation::Rotate), "↻", "Rotate"),
                        (Some(Operation::Scale), "↗", "Scale"),
                    ] {
                        if ui
                            .selectable_label(
                                self.tool == operation && self.live_tool.is_none(),
                                icon,
                            )
                            .on_hover_text(label)
                            .clicked()
                        {
                            self.tool = operation;
                            self.live_tool = None;
                        }
                    }
                    ui.separator();
                    for (operation, icon, label) in [
                        (Operation::Move, "↔", "Grab live"),
                        (Operation::Rotate, "↶", "Tip live"),
                    ] {
                        if ui
                            .selectable_label(self.live_tool == Some(operation), icon)
                            .on_hover_text(label)
                            .clicked()
                        {
                            self.tool = None;
                            self.live_tool = Some(operation);
                        }
                    }
                } else if ui
                    .button("E")
                    .on_hover_text("Return to Object Mode · Tab")
                    .clicked()
                {
                    self.toggle_edit();
                }
            });
        }
        let mut before = None;
        if self.sidebar || self.shading {
            let bounds = egui::Rect::from_min_size(
                egui::pos2(rect.right() - 252.0, rect.top() + 52.0),
                egui::vec2(238.0, 226.0),
            );
            let mut side = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("viewport settings")
                    .max_rect(bounds),
            );
            panel(BAR, 10).show(&mut side, |ui| {
                if self.shading {
                    section(ui, "Viewport shading");
                    let old = self.mode;
                    for mode in [
                        Display::Solid,
                        Display::Surface,
                        Display::RayTraced,
                        Display::Particles,
                    ] {
                        if ui
                            .selectable_value(&mut self.mode, mode, mode.label())
                            .clicked()
                        {
                            self.shading = false;
                        }
                    }
                    self.dirty |= old != self.mode;
                }
                if self.sidebar {
                    section(ui, "Transform");
                    let prior = self.scene.clone();
                    if let Some(object) = self
                        .scene
                        .objects
                        .iter_mut()
                        .find(|o| Some(o.id) == self.selected)
                    {
                        if transform_fields(ui, object) {
                            before = Some(prior);
                        }
                    } else {
                        ui.label("Select an object.");
                    }
                }
            });
        }
        self.remember_inspector(ui.ctx(), before);
    }

    pub(super) fn remember_inspector(&mut self, ctx: &egui::Context, before: Option<Scene>) {
        if let Some(before) = before {
            self.inspector_edit.get_or_insert(before);
            self.dirty = true;
        }
        if !ctx.input(|i| i.pointer.any_down())
            && let Some(before) = self.inspector_edit.take()
        {
            self.remember(before);
        }
    }

    fn handles(&self, rect: egui::Rect) -> Option<(egui::Pos2, [egui::Pos2; 3])> {
        self.tool?;
        let object = self
            .selection()
            .filter(|o| o.visible && o.enabled && o.selectable)?;
        let center = input::project(self.camera, rect, object.position)?;
        let size = self.camera.distance * 0.085;
        let ends = input::AXES.map(|axis| {
            input::project(self.camera, rect, object.position + axis * size).unwrap_or(center)
        });
        Some((center, ends))
    }

    pub(super) fn start_gizmo(&mut self, rect: egui::Rect, pointer: egui::Pos2) {
        let Some((center, ends)) = self.handles(rect) else {
            return;
        };
        let axis = ends
            .iter()
            .enumerate()
            .filter(|(_, p)| p.distance(center) > 13.0 && p.distance(pointer) < 13.0)
            .min_by(|(_, a), (_, b)| a.distance(pointer).total_cmp(&b.distance(pointer)))
            .map(|(axis, _)| axis);
        if let Some(axis) = axis
            && let Some(mut transform) =
                Transform::new(&self.scene, self.selected.unwrap(), self.tool.unwrap())
        {
            transform.axis = Some(axis);
            self.transform = Some(transform);
            self.gizmo_drag = true;
        }
    }

    pub(super) fn paint_gizmo(&self, rect: egui::Rect, painter: &egui::Painter) {
        let Some((center, ends)) = self.handles(rect) else {
            return;
        };
        for (end, color) in ends.into_iter().zip([
            Color32::from_rgb(243, 111, 105),
            Color32::from_rgb(134, 208, 140),
            Color32::from_rgb(113, 162, 252),
        ]) {
            if end.distance(center) < 13.0 {
                continue;
            }
            let stroke = Stroke::new(2.0_f32, color);
            match self.tool {
                Some(Operation::Move) => painter.arrow(center, end - center, stroke),
                Some(Operation::Rotate) => {
                    painter.line_segment([center, end], stroke);
                    painter.circle_stroke(end, 6.0, stroke);
                }
                Some(Operation::Scale) => {
                    painter.line_segment([center, end], stroke);
                    painter.rect_filled(
                        egui::Rect::from_center_size(end, egui::vec2(9.0, 9.0)),
                        1.0,
                        color,
                    );
                }
                None => {}
            }
        }
    }
}

fn frame_input(ui: &mut egui::Ui, label: &str, value: &mut u32, min: u32) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(value).range(min..=u32::MAX))
            .changed()
    })
    .inner
}

fn volume_input(ui: &mut egui::Ui, label: &str, value: &mut f32, max: f32) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::DragValue::new(value)
                .speed(0.5)
                .max_decimals(2)
                .range(0.0..=max),
        )
        .changed()
    })
    .inner
}

fn transform_fields(ui: &mut egui::Ui, object: &mut Object) -> bool {
    fn row(
        ui: &mut egui::Ui,
        name: &str,
        values: &mut [f32; 3],
        bounds: [(f32, f32); 3],
        speed: f32,
    ) -> bool {
        ui.label(RichText::new(name).color(MUTED).small());
        ui.horizontal(|ui| {
            let mut changed = false;
            for (i, value) in values.iter_mut().enumerate() {
                changed |= ui
                    .add(
                        egui::DragValue::new(value)
                            .range(bounds[i].0..=bounds[i].1)
                            .speed(speed)
                            .prefix(["X ", "Y ", "Z "][i])
                            .max_decimals(2),
                    )
                    .changed();
            }
            changed
        })
        .inner
    }
    let mut location = input::AXES.map(|axis| object.position.dot(axis));
    let (x, y, z) = object.rotation.to_euler(glam::EulerRot::XYZ);
    let mut angles = [x.to_degrees(), -z.to_degrees(), y.to_degrees()];
    let mut dimensions = [
        object.half.x * 2.0,
        object.half.z * 2.0,
        object.half.y * 2.0,
    ];
    let moved = row(
        ui,
        "Location · m",
        &mut location,
        [(-1.9, 1.9), (-1.3, 1.3), (0.05, 2.72)],
        0.015,
    );
    let rotated = row(ui, "Rotation · °", &mut angles, [(-180.0, 180.0); 3], 0.8);
    let min = if object.kind == Kind::Tank {
        0.40
    } else {
        0.09
    };
    let sized = matches!(object.kind, Kind::Tank | Kind::Box)
        && row(
            ui,
            "Dimensions · m",
            &mut dimensions,
            [(min, 3.7), (min, 2.5), (min, 2.4)],
            0.015,
        );
    if moved {
        object.position = Vec3::new(location[0], location[2], -location[1]);
    }
    if rotated {
        object.rotation = Quat::from_euler(
            glam::EulerRot::XYZ,
            angles[0].to_radians(),
            angles[2].to_radians(),
            -angles[1].to_radians(),
        );
    }
    if sized {
        object.half = Vec3::new(dimensions[0], dimensions[2], dimensions[1]) * 0.5;
    }
    moved || rotated || sized
}
