use std::time::{Duration, Instant};

use eframe::egui_wgpu::{self, wgpu};
use glam::{Quat, Vec3};

use crate::{
    fluid::{DT, END, Fluid, ORIGIN, QUALITIES},
    input::{Operation, Transform},
    scene::{Kind, Object, Preset, Scene},
    viewport::{Camera, Display, Renderer, ViewSettings},
};

#[test]
fn inspector_stays_at_dragged_width_when_content_overflows() {
    use eframe::egui::{self, Event, PointerButton};

    let ctx = egui::Context::default();
    let id = egui::Id::new("inspector");
    let frame = |width: f32, events| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 800.0),
            )),
            events,
            ..Default::default()
        };
        let mut result = (0.0, 0.0);
        let _ = ctx.run(input, |ctx| {
            crate::show_dock(
                ctx,
                egui::SidePanel::left("scene").exact_width(204.0),
                14,
                |_| {},
            );
            crate::show_dock(ctx, crate::inspector_dock(ctx), 16, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.allocate_space(egui::vec2(2_000.0, 30.0));
                });
            });
            let state = egui::containers::panel::PanelState::load(ctx, id).unwrap();
            result = (state.size().x, ctx.available_rect().width());
        });
        result
    };
    let pointer = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };

    let width = 1920.0;
    assert_eq!(frame(width, vec![]).0, 260.0);
    let divider = egui::pos2(width - 260.0, 300.0);
    let narrower = divider + egui::vec2(30.0, 0.0);
    frame(width, vec![Event::PointerMoved(divider)]);
    frame(width, vec![pointer(divider, true)]);
    frame(width, vec![Event::PointerMoved(narrower)]);
    frame(width, vec![pointer(narrower, false)]);
    for _ in 0..12 {
        assert_eq!(frame(width, vec![]).0, 230.0);
    }

    let old_bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1_700.0, 800.0));
    ctx.data_mut(|data| {
        data.insert_persisted(id, egui::containers::panel::PanelState { rect: old_bounds });
    });
    let (inspector, viewport) = frame(width, vec![]);
    assert_eq!(inspector, 400.0);
    assert!(viewport >= 360.0);
    let (inspector, viewport) = frame(950.0, vec![]);
    assert!(inspector <= 400.0 && viewport >= 360.0);
}

#[test]
fn pressure_projection_advects_the_pour_deterministically() {
    let scene = Scene::preset(Preset::Pour);
    let q = QUALITIES[1];
    let mut a = Fluid::new(&scene, q);
    let mut b = Fluid::new(&scene, q);
    let initial = a.len();
    for _ in 0..36 {
        a.step(&scene, q);
        b.step(&scene, q);
    }
    assert_eq!(a.snapshot(), b.snapshot());
    assert!(a.emitted > 350);
    assert_eq!(a.len() as u64, initial as u64 + a.emitted);
    assert!(
        a.snapshot()
            .iter()
            .any(|p| p[0] < -0.2 && p[1] < 1.9 && p[3] > 0.75)
    );
    assert_bounded(&a);
}

#[test]
fn whitewater_reacts_to_impact_without_changing_liquid_volume_or_motion() {
    let quality = QUALITIES[1];
    let scene = Scene::preset(Preset::Pour);
    let mut quiet_scene = scene.clone();
    quiet_scene.objects[0].flow *= 0.08;
    quiet_scene.objects[0].speed *= 0.08;
    let mut disabled = scene.clone();
    disabled.whitewater.amount = 0.0;
    let mut fluid = Fluid::new(&scene, quality);
    let mut quiet = Fluid::new(&quiet_scene, quality);
    let mut no_whitewater = Fluid::new(&disabled, quality);
    let mut phases = [0; 3];
    for _ in 0..180 {
        fluid.step(&scene, quality);
        quiet.step(&quiet_scene, quality);
        no_whitewater.step(&disabled, quality);
        for p in fluid.secondary_snapshot() {
            phases[p[3].floor() as usize - 1] += 1;
        }
    }
    assert!(phases.into_iter().all(|count| count > 20), "{phases:?}");
    assert!(fluid.secondary_snapshot().len() > quiet.secondary_snapshot().len() * 3);
    assert!(no_whitewater.secondary_snapshot().is_empty());
    assert_eq!(fluid.snapshot(), no_whitewater.snapshot());
}

#[test]
fn settled_cup_and_honey_do_not_generate_foam_piles() {
    let quality = QUALITIES[1];
    let station = Scene::preset(Preset::Station);
    let honey = Scene::preset(Preset::Honey);
    let mut still = Fluid::new(&station, quality);
    let mut viscous = Fluid::new(&honey, quality);
    let mut honey_peak = 0;
    let mut settled_peak = 0;
    for frame in 0..460 {
        still.step(&station, quality);
        viscous.step(&honey, quality);
        if frame > 180 {
            settled_peak = settled_peak.max(still.secondary_snapshot().len());
        }
        honey_peak = honey_peak.max(viscous.secondary_snapshot().len());
    }
    println!("Secondary particle peaks: settled cup {settled_peak}, honey {honey_peak}");
    assert!(settled_peak < 8 && honey_peak < 8);
}

#[test]
fn live_geometry_pours_into_the_station_and_fields_move_water() {
    use crate::scene::{FieldKind, FlowMode, FluidRole};
    let quality = QUALITIES[0];
    let mut station = Scene::preset(Preset::Station);
    let mut fluid = Fluid::new(&station, quality);
    let initial = fluid.len();
    for tick in 0_u32..360 {
        station.objects[1].rotation =
            Quat::from_rotation_z((tick.saturating_sub(55_u32) as f32 * 0.023).min(2.2));
        fluid.step(&station, quality);
    }
    assert_eq!(fluid.len(), initial);
    assert!(fluid.snapshot().iter().filter(|p| p[1] < 0.91).count() > initial / 2);

    let mut scene = Scene::preset(Preset::Empty);
    let mut domain = Object::new(1, Kind::Box, Vec3::new(0.0, 0.85, 0.0));
    domain.half = Vec3::new(1.25, 0.8, 0.9);
    let mut source = Object::new(2, Kind::Box, Vec3::new(-0.44, 0.63, 0.0));
    source.half = Vec3::new(0.26, 0.22, 0.38);
    source.flow_mode = FlowMode::Geometry;
    let mut field = Object::new(3, Kind::Field, source.position);
    field.field.kind = FieldKind::Directional;
    field.field.strength = 35.0;
    field.rotation = Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
    scene.objects.extend([domain, source, field]);
    scene.assign_role(1, FluidRole::Domain);
    scene.assign_role(2, FluidRole::Flow);
    let mut sim = Fluid::new(&scene, quality);
    let initial = sim.len();
    assert!(initial > 400);
    for _ in 0..40 {
        sim.step(&scene, quality);
    }
    let points = sim.snapshot();
    assert!(
        points
            .iter()
            .all(|p| p[0].abs() < 1.25 && p[2].abs() < 0.90)
    );
    assert!(points.iter().map(|p| p[0]).sum::<f32>() / points.len() as f32 > 0.0);
    scene.objects[2].enabled = false;
    scene.objects[1].position = Vec3::new(0.7, 0.22, 0.0);
    scene.objects[1].radius = 0.55;
    scene.objects[1].flow_mode = FlowMode::Outflow;
    for _ in 0..55 {
        sim.step(&scene, quality);
    }
    assert!(sim.drained > 0);
    assert_eq!(sim.len() as u64 + sim.drained, initial as u64);
}

#[test]
fn released_dam_flows_and_drains_account_for_removed_volume() {
    let mut scene = Scene::preset(Preset::Dam);
    let q = QUALITIES[1];
    let mut fluid = Fluid::new(&scene, q);
    let initial = fluid.len();
    scene.objects[1].enabled = false;
    for _ in 0..96 {
        fluid.step(&scene, q);
    }
    assert!(fluid.snapshot().iter().filter(|p| p[0] > 0.2).count() > 150);
    assert_eq!(fluid.len(), initial);
    let mut drain = Object::new(3, Kind::Drain, Vec3::new(0.62, 0.33, 0.0));
    drain.radius = 0.55;
    scene.objects.push(drain);
    for _ in 0..24 {
        fluid.step(&scene, q);
    }
    assert!(fluid.drained > 50);
    assert_eq!(fluid.len() as u64 + fluid.drained, initial as u64);
    assert_bounded(&fluid);
}

#[test]
fn closed_dam_holds_its_water_level() {
    let scene = Scene::preset(Preset::Dam);
    let quality = QUALITIES[1];
    let mut fluid = Fluid::new(&scene, quality);
    let percentile = |particles: &[[f32; 4]]| {
        let mut heights: Vec<_> = particles.iter().map(|p| p[1]).collect();
        let index = heights.len() * 95 / 100;
        *heights.select_nth_unstable_by(index, f32::total_cmp).1
    };
    let initial = fluid.snapshot();
    let seeded_level = percentile(&initial);
    for _ in 0..720 {
        fluid.step(&scene, quality);
    }
    let settled = fluid.snapshot();
    let level = percentile(&settled);
    println!(
        "Closed dam at 8 s: {:.3} m → {:.3} m; {:.1} L; {} particles",
        seeded_level,
        level,
        fluid.initial_liters,
        settled.len()
    );
    assert_eq!(initial.len(), settled.len());
    assert!(settled.iter().all(|p| p[0] < -0.095));
    assert!(
        seeded_level - level < 0.085,
        "The closed compartment compressed."
    );
    assert_bounded(&fluid);
}

#[test]
fn liters_and_flow_schedule_drive_the_solver() {
    let mut scene = Scene::preset(Preset::Pour);
    let quality = QUALITIES[0];
    scene.fill_liters = Some(70.0);
    let source = &mut scene.objects[0];
    source.schedule.start = 18;
    source.schedule.stop = Some(72);
    source.schedule.ramp = 0.10;
    source.schedule.liters = Some(12.0);
    let mut fluid = Fluid::new(&scene, quality);
    assert!((fluid.initial_liters - 70.0).abs() < 0.02);
    for _ in 0..18 {
        fluid.step(&scene, quality);
    }
    assert_eq!(fluid.emitted, 0);
    for _ in 18..72 {
        fluid.step(&scene, quality);
    }
    let emitted = fluid.emitted;
    let volume = emitted as f32 * fluid.particle_volume * 1000.0;
    assert!(volume <= 12.0 && volume > 12.0 - fluid.particle_volume * 1000.0 - 0.001);
    for _ in 72..90 {
        fluid.step(&scene, quality);
    }
    assert_eq!(fluid.emitted, emitted);
}

#[test]
fn topology_tools_create_valid_mesh_colliders() {
    use crate::{
        collider::MeshCollider,
        mesh::{CutSide, Primitive},
    };
    let mut cube = Primitive::Cube.geometry();
    let ring = cube.loop_ring((0, 1)).unwrap();
    assert!(cube.cut_ring(&ring, 1, 0.0).is_some());
    assert!(cube.valid() && cube.closed());
    assert!(cube.bisect(Vec3::X, 0.4, CutSide::Positive, true));
    assert!(cube.valid() && cube.closed());
    let mut object = Object::new(1, Kind::Box, Vec3::ZERO);
    object.half = Vec3::ONE;
    object.mesh = Some(cube);
    let collider = MeshCollider::new(&object);
    assert!(collider.solid(Vec3::new(0.7, 0.12, 0.25)));
    assert!(!collider.solid(Vec3::new(0.12, 0.12, 0.25)));
    let mut plane = Primitive::Plane.geometry();
    assert!(plane.knife(0, [((0, 1), 0.5), ((2, 3), 0.5)]));
    assert_eq!(plane.faces.len(), 2);
    assert!(plane.valid());
    for primitive in Primitive::ALL {
        let mesh = primitive.geometry();
        assert!(
            mesh.valid() && !mesh.triangles().is_empty(),
            "{primitive:?}"
        );
    }
    let tank = crate::mesh::Mesh::tank_shell(Vec3::new(0.9, 0.5, 0.7));
    assert!(tank.closed());

    let mut model = Primitive::Cube.geometry();
    let ring = model.loop_ring((0, 1)).unwrap();
    let cut = model.cut_ring(&ring, 3, 0.5).unwrap();
    assert_eq!(cut.len(), 12);
    assert!(model.closed() && model.valid());
    assert!(
        model.points[8..]
            .iter()
            .all(|p| (p.x.abs() - 0.25).abs() < 0.001 || (p.x - 0.75).abs() < 0.001)
    );
    let mut beveled = Primitive::Cube.geometry();
    assert!(beveled.bevel_edge((0, 1), 0.18, 4));
    assert!(beveled.closed() && beveled.valid());
    assert_eq!(beveled.faces.len(), 18);
    let mut extruded = Primitive::Cube.geometry();
    assert!(extruded.extrude(&[1].into_iter().collect(), Vec3::Y * 0.25));
    assert!(extruded.closed() && extruded.valid());
    assert!(extruded.points.iter().any(|p| (p.y - 1.25).abs() < 0.001));
}

#[test]
fn slow_tap_detaches_separate_drops_and_honors_stop_frame() {
    let scene = Scene::preset(Preset::Wetting);
    let quality = QUALITIES[0];
    let mut fluid = Fluid::new(&scene, quality);
    let mut bursts = 0;
    let mut previous = 0;
    for tick in 0..325 {
        fluid.step(&scene, quality);
        if fluid.emitted != previous {
            assert!(tick < 270 && fluid.emitted - previous >= 6);
            bursts += 1;
            previous = fluid.emitted;
        }
    }
    assert!(bursts >= 10 && bursts < 40);
    assert_eq!(fluid.len() as u64, fluid.emitted);
    assert_bounded(&fluid);
}

#[test]
fn honey_follows_the_moving_spoon_with_some_runoff() {
    let scene = Scene::preset(Preset::Honey);
    let quality = QUALITIES[1];
    let mut wet = Fluid::new(&scene, quality);
    let mut dry_scene = scene.clone();
    dry_scene.objects[1].wetting = 0.0;
    let mut dry = Fluid::new(&dry_scene, quality);
    for _ in 0..400 {
        wet.step(&scene, quality);
        dry.step(&dry_scene, quality);
    }
    let tilted = scene.at_frame(wet.ticks);
    assert!(
        tilted.objects[1]
            .position
            .distance(scene.objects[1].position)
            > 0.35
    );
    let spoon = crate::collider::MeshCollider::new(&tilted.objects[1]);
    let on_spoon = |f: &Fluid| {
        f.snapshot()
            .iter()
            .filter(|p| {
                spoon
                    .tree
                    .nearest(Vec3::from_slice(&p[..3]), 0.095)
                    .is_some()
            })
            .count()
    };
    let wet_count = on_spoon(&wet);
    let dry_count = on_spoon(&dry);
    let runoff = wet.snapshot().iter().filter(|p| p[1] < 0.73).count();
    println!("Spoon: {wet_count} wet vs {dry_count} dry near surface, {runoff} runoff");
    assert!(wet_count > dry_count + 20);
    assert!(runoff > 10);
    assert_eq!(wet.len(), dry.len());
}

#[test]
fn oil_rises_after_impact_without_changing_liquid_volume() {
    let scene = Scene::preset(Preset::Layers);
    let quality = QUALITIES[1];
    let mut fluid = Fluid::new(&scene, quality);
    let initial = fluid.len();
    let medians = |fluid: &Fluid| {
        let mut levels = [Vec::new(), Vec::new()];
        for p in fluid.snapshot() {
            levels[usize::from(p[3] < 0.0)].push(p[1]);
        }
        levels.map(|mut v| {
            v.sort_by(f32::total_cmp);
            v[v.len() / 2]
        })
    };
    for _ in 0..270 {
        fluid.step(&scene, quality);
    }
    let impact = medians(&fluid);
    for _ in 270..495 {
        fluid.step(&scene, quality);
    }
    let settled = medians(&fluid);
    println!(
        "Oil median after impact {:.3} m → {:.3} m, water {:.3} m",
        impact[1], settled[1], settled[0]
    );
    assert!(settled[1] > impact[1] + 0.13 && settled[1] > settled[0] + 0.12);
    assert_eq!(fluid.len() as u64, initial as u64 + fluid.emitted);
    assert_bounded(&fluid);
}

#[test]
fn timed_gate_lifts_through_the_water_before_opening() {
    let mut scene = Scene::preset(Preset::Dam);
    scene.gate_frame = Some(30);
    scene.gate_lift_frames = 65;
    let quality = QUALITIES[0];
    let mut fluid = Fluid::new(&scene, quality);
    for _ in 0..29 {
        fluid.step(&scene, quality);
    }
    assert!(fluid.snapshot().iter().all(|p| p[0] < 0.0));
    let mid = scene.at_frame(59);
    assert!(
        mid.objects[1].enabled && mid.objects[1].position.y > scene.objects[1].position.y + 0.4
    );
    for _ in 29..175 {
        fluid.step(&scene, quality);
    }
    assert!(!scene.at_frame(fluid.ticks).objects[1].enabled);
    assert!(fluid.snapshot().iter().filter(|p| p[0] > 0.25).count() > 300);
}

#[test]
fn open_mesh_collision_keeps_water_above_edited_surface() {
    let mut scene = Scene::preset(Preset::Slosh);
    scene.objects.clear();
    scene.fill = Some(crate::scene::Fill {
        min: Vec3::new(-0.30, 1.0, -0.27),
        max: Vec3::new(0.30, 1.3, 0.27),
    });
    let mut platform = Object::new(1, Kind::Box, Vec3::new(0.0, 0.66, 0.0));
    platform.half = Vec3::new(1.85, 0.3, 1.25);
    platform.mesh = Some(crate::mesh::Primitive::Plane.geometry());
    scene.objects.push(platform);
    let quality = QUALITIES[0];
    let mut fluid = Fluid::new(&scene, quality);
    for _ in 0..80 {
        fluid.step(&scene, quality);
    }
    let particles = fluid.snapshot();
    let above = particles.iter().filter(|p| p[1] >= 0.66).count();
    assert!(
        above > particles.len() * 97 / 100,
        "{above} above of {}",
        particles.len()
    );
}

#[test]
fn hdr_and_exr_keep_linear_highlights() {
    crate::environment::check_formats().unwrap();
}

#[test]
fn tilting_tank_moves_the_water_and_keeps_most_inside() {
    let mut scene = Scene::preset(Preset::Slosh);
    let q = QUALITIES[1];
    let mut fluid = Fluid::new(&scene, q);
    let initial = fluid.snapshot();
    for tick in 0..105 {
        scene.objects[0].rotation = Quat::from_rotation_z((tick as f32 * DT * 4.6).sin() * 0.24);
        fluid.step(&scene, q);
    }
    let tank = &scene.objects[0];
    let water = fluid.snapshot();
    let inside = water
        .iter()
        .filter(|p| {
            let local = tank.rotation.conjugate() * (Vec3::from_slice(&p[..3]) - tank.position);
            local.x.abs() < tank.half.x && local.z.abs() < tank.half.z && local.y > -tank.half.y
        })
        .count();
    assert_eq!(water.len(), initial.len());
    assert!(
        inside > water.len() * 96 / 100,
        "{inside} of {} stayed in tank",
        water.len()
    );
    assert!(
        water
            .iter()
            .zip(initial)
            .filter(|(a, b)| (a[0] - b[0]).abs() > 0.1)
            .count()
            > water.len() / 4
    );
    assert_bounded(&fluid);
}

fn assert_bounded(fluid: &Fluid) {
    for p in fluid.snapshot() {
        let pos = Vec3::from_slice(&p[..3]);
        assert!(pos.is_finite() && p[3].is_finite());
        assert!(pos.cmpge(ORIGIN).all() && pos.cmple(END).all());
    }
}

#[test]
fn scene_round_trip_and_numeric_transforms() {
    let mut scene = Scene::preset(Preset::Pour);
    let mut transform = Transform::new(&scene, 1, Operation::Move).unwrap();
    transform.axis_key(0);
    transform.number = "0.5".into();
    transform.apply(&mut scene.objects[0], Camera::default(), 720.0);
    assert!((scene.objects[0].position.x - transform.original.position.x - 0.5).abs() < 0.0001);
    let data = serde_json::to_vec(&scene).unwrap();
    let mut loaded: Scene = serde_json::from_slice(&data).unwrap();
    loaded.validate().unwrap();
    assert_eq!(loaded.objects[0].position, scene.objects[0].position);
    loaded.objects[1].id = 1;
    assert!(loaded.validate().is_err());
    let mut legacy: serde_json::Value = serde_json::from_slice(&data).unwrap();
    legacy.as_object_mut().unwrap().remove("lighting");
    for object in legacy["objects"].as_array_mut().unwrap() {
        let object = object.as_object_mut().unwrap();
        object.remove("visible");
        object.remove("selectable");
    }
    let mut old_scene: Scene = serde_json::from_value(legacy).unwrap();
    old_scene.validate().unwrap();
    assert!(
        old_scene.lighting.caustics
            && old_scene.objects[0].visible
            && old_scene.objects[0].selectable
    );
}

#[test]
fn wgsl_validates() {
    let source = crate::viewport::SHADER;
    let shader = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&shader)
    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
}

#[test]
fn surface_reconstruction_joins_liquid_without_drawing_isolated_particles() {
    let scene = Scene::preset(Preset::Slosh);
    let mut field = crate::surface::Surface::new();
    field.rebuild(&scene, &[[0.0, 1.10, 0.0, 0.0]]);
    assert!(field.occupancy.iter().all(|&b| b == 0));
    let mut points = Vec::new();
    for z in -2..=2 {
        for y in -2..=2 {
            for x in -2..=2 {
                points.push([
                    x as f32 * 0.051,
                    1.10 + y as f32 * 0.051,
                    z as f32 * 0.051,
                    0.0,
                ]);
            }
        }
    }
    field.rebuild(&scene, &points);
    assert!(field.occupancy.iter().any(|&b| b != 0));
    assert!(
        field
            .density
            .iter()
            .filter(|&&b| b[0] >= (crate::surface::ISO * 255.0) as u8)
            .count()
            > 60
    );
}

#[test]
#[ignore = "Uses the Windows software adapter for bounded viewport verification"]
fn software_renders_liquid_surface_and_particle_view() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..Default::default()
    });
    let adapter = finish(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: true,
        ..Default::default()
    }))
    .unwrap();
    assert_eq!(adapter.get_info().device_type, wgpu::DeviceType::Cpu);
    let (device, queue) = finish(adapter.request_device(&Default::default())).unwrap();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let renderer = egui_wgpu::Renderer::new(&device, format, Default::default());
    let state = egui_wgpu::RenderState {
        adapter,
        available_adapters: vec![],
        device: device.clone(),
        queue: queue.clone(),
        target_format: format,
        renderer: std::sync::Arc::new(eframe::egui::mutex::RwLock::new(renderer)),
    };
    let mut render = Renderer::new(state.clone());
    const SIZE: [u32; 2] = [640, 448];
    render.resize(SIZE);
    let mut scene = Scene::preset(Preset::Pour);
    let q = QUALITIES[1];
    let mut fluid = Fluid::new(&scene, q);
    let in_tank = |points: &[[f32; 4]]| {
        points
            .iter()
            .filter(|p| {
                p[0] > -0.06 && p[0] < 1.52 && p[1] > 0.23 && p[1] < 1.11 && p[2].abs() < 0.59
            })
            .count()
    };
    let initial_tank = in_tank(&fluid.snapshot());
    let start = Instant::now();
    const STEPS: u32 = 156;
    let mut history = vec![(0.0, fluid.snapshot())];
    for tick in 1..=STEPS {
        fluid.step(&scene, q);
        if tick % 13 == 0 {
            history.push((fluid.time, fluid.snapshot()));
        }
    }
    let elapsed = start.elapsed();
    println!(
        "{STEPS} pour steps took {:.2}s for {} particles",
        elapsed.as_secs_f32(),
        fluid.len()
    );
    let p = fluid.snapshot();
    let secondary = fluid.secondary_snapshot();
    assert!(
        in_tank(&p) > initial_tank + 150,
        "The catch tank did not receive the pour"
    );
    let mut settings = ViewSettings {
        camera: Camera::default(),
        selected: Some(1),
        mode: Display::Surface,
        detail: 1,
        surface_revision: 0,
    };
    let mut capture = |scene: &Scene, particles: &[[f32; 4]], settings| {
        render.render(scene, particles, settings);
        read_rgba(&device, &queue, render.output(), SIZE)
    };
    let empty = capture(&scene, &[], settings);
    settings.surface_revision += 1;
    let surface = capture(&scene, &p, settings);
    settings.mode = Display::RayTraced;
    let ray_traced = capture(&scene, &p, settings);
    let mut checker = scene.clone();
    checker.lighting.checker = true;
    let refracted = capture(&checker, &p, settings);
    let mut no_focus = checker.clone();
    no_focus.lighting.caustics = false;
    let without_caustics = capture(&no_focus, &p, settings);
    no_focus.liquid.ior = 1.0;
    let straight = capture(&no_focus, &p, settings);
    settings.mode = Display::Particles;
    let particles = capture(&scene, &p, settings);
    settings.camera.ortho = true;
    let ortho = capture(&scene, &p, settings);
    let different = |a: &[u8], b: &[u8]| {
        a.chunks_exact(4)
            .zip(b.chunks_exact(4))
            .filter(|(a, b)| a[1].abs_diff(b[1]) > 12)
            .count()
    };
    assert!(different(&surface, &empty) > 3000);
    assert!(different(&surface, &particles) > 2000);
    assert!(different(&surface, &ray_traced) > 120);
    assert!(
        different(&refracted, &without_caustics) > 35,
        "Focused light did not reach the receivers"
    );
    assert!(
        different(&without_caustics, &straight) > 500,
        "The IOR did not alter the 3D refraction"
    );
    assert!(different(&particles, &ortho) > 2000);
    drop(capture);
    render.render_live(&scene, &p, &secondary, settings);
    let particle_inspection = read_rgba(&device, &queue, render.output(), SIZE);
    assert!(different(&particle_inspection, &ortho) < 50);
    settings.mode = Display::Surface;
    settings.camera = Camera::default();
    render.render_live(&scene, &p, &secondary, settings);
    let splash = read_rgba(&device, &queue, render.output(), SIZE);
    assert!(
        different(&splash, &surface) > 12,
        "Secondary particles did not change the render"
    );
    render.clip_plane = Some((Vec3::Z, 0.0));
    render.render_live(&scene, &p, &secondary, settings);
    let sliced = read_rgba(&device, &queue, render.output(), SIZE);
    assert!(
        different(&splash, &sliced) > 1000,
        "Cross section did not reveal the scene interior"
    );
    render.clip_plane = None;
    let until = Instant::now() + Duration::from_secs(10);
    while !render.panorama_status().ends_with(" HDR") && Instant::now() < until {
        render.poll_environment(&scene.lighting);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(render.panorama_status(), "2048 × 1024 HDR");
    settings.camera = Camera::default();
    settings.mode = Display::Surface;
    if let Some(path) = std::env::var_os("MLSB_SPLASH_PREVIEW") {
        let mut preview = Fluid::new(&scene, q);
        for _ in 0..511 {
            preview.step(&scene, q);
        }
        let mut view = settings;
        view.surface_revision = 999;
        render.render_live(
            &scene,
            &preview.snapshot(),
            &preview.secondary_snapshot(),
            view,
        );
        let pixels = read_rgba(&device, &queue, render.output(), SIZE);
        image::save_buffer(path, &pixels, SIZE[0], SIZE[1], image::ColorType::Rgba8).unwrap();
    }
    render.render(&scene, &p, settings);
    let garden = read_rgba(&device, &queue, render.output(), SIZE);
    assert!(
        different(&surface, &garden) > 3000,
        "The HDRI did not change the render."
    );
    let ramp = &mut scene.objects[1];
    ramp.mesh = Some(crate::mesh::Primitive::Ramp.geometry());
    render.render(&scene, &p, settings);
    let with_mesh = read_rgba(&device, &queue, render.output(), SIZE);
    assert!(
        different(&garden, &with_mesh) > 100,
        "The edited mesh did not change the render."
    );
    if let Some(path) = std::env::var_os("MLSB_PREVIEW") {
        image::save_buffer(path, &garden, SIZE[0], SIZE[1], image::ColorType::Rgba8).unwrap();
    }
    if let Some(path) = std::env::var_os("MLSB_OPTICS_PREVIEW") {
        image::save_buffer(path, &refracted, SIZE[0], SIZE[1], image::ColorType::Rgba8).unwrap();
    }
    let layers = Scene::preset(Preset::Layers);
    let mut phases = Fluid::new(&layers, q);
    for _ in 0..495 {
        phases.step(&layers, q);
    }
    let two = phases.snapshot();
    let mut one = two.clone();
    for particle in &mut one {
        particle[3] = particle[3].abs();
    }
    settings.surface_revision += 1;
    render.render(&layers, &two, settings);
    let separate = read_rgba(&device, &queue, render.output(), SIZE);
    settings.surface_revision += 1;
    render.render(&layers, &one, settings);
    let clear = read_rgba(&device, &queue, render.output(), SIZE);
    assert!(
        different(&separate, &clear) > 120,
        "The viewport did not show the oil layer"
    );
    if let Some(path) = std::env::var_os("MLSB_LAYERS_PREVIEW") {
        image::save_buffer(path, &separate, SIZE[0], SIZE[1], image::ColorType::Rgba8).unwrap();
    }
    if let Some(path) = std::env::var_os("MLSB_HONEY_PREVIEW") {
        let honey = Scene::preset(Preset::Honey);
        let mut sim = Fluid::new(&honey, q);
        for _ in 0..248 {
            sim.step(&honey, q);
        }
        settings.surface_revision += 1;
        render.render(&honey.at_frame(sim.ticks), &sim.snapshot(), settings);
        let pixels = read_rgba(&device, &queue, render.output(), SIZE);
        image::save_buffer(path, &pixels, SIZE[0], SIZE[1], image::ColorType::Rgba8).unwrap();
    }
    render_interface(state, p, fluid.time, elapsed / STEPS, history);
}

fn render_interface(
    state: egui_wgpu::RenderState,
    particles: Vec<[f32; 4]>,
    time: f32,
    solve: Duration,
    history: Vec<(f32, Vec<[f32; 4]>)>,
) {
    use crate::App;
    use eframe::egui::{self, Key};
    let ctx = egui::Context::default();
    let logo = image::load_from_memory(include_bytes!("../../logo.png"))
        .unwrap()
        .into_rgba8();
    let mut app = App::new(&ctx, state.clone(), &logo).unwrap();
    app.set_preset(Preset::Pour);
    app.snapshot.epoch = app.epoch;
    app.snapshot.particles = particles.into();
    app.snapshot.time = time;
    app.snapshot.step_time = solve;
    app.cache.extend(
        history
            .into_iter()
            .map(|(time, particles)| crate::worker::Snapshot {
                epoch: app.epoch,
                time,
                particles: particles.into(),
                scene: Some(app.scene.clone()),
                ..Default::default()
            }),
    );
    const SIZE: [u32; 2] = [1280, 800];
    let screen_rect = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
    ));
    let raw = |events| egui::RawInput {
        screen_rect,
        events,
        ..Default::default()
    };
    let first = ctx.run(raw(vec![]), |ctx| app.frame(ctx));
    let mut output = ctx.run(raw(vec![]), |ctx| app.frame(ctx));
    let mut textures = first.textures_delta;
    textures.append(output.textures_delta);
    output.textures_delta = textures;
    assert!(app.renderer.texture_id().is_some());
    let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: SIZE,
        pixels_per_point: output.pixels_per_point,
    };
    let device = &state.device;
    let queue = &state.queue;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Software UI verification"),
        size: wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut renderer = state.renderer.write();
        for (id, image) in &output.textures_delta.set {
            renderer.update_texture(device, queue, *id, image);
        }
        assert!(
            renderer
                .update_buffers(device, queue, &mut encoder, &primitives, &screen)
                .is_empty()
        );
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            })
            .forget_lifetime();
        renderer.render(&mut pass, &primitives, &screen);
    }
    queue.submit([encoder.finish()]);
    let image = read_rgba(device, queue, &texture, SIZE);
    assert!(
        image
            .chunks_exact(4)
            .filter(|p| p[0] > 150 && p[1] > 160)
            .count()
            > 1000
    );
    if let Some(path) = std::env::var_os("MLSB_UI_PREVIEW") {
        image::save_buffer(path, &image, SIZE[0], SIZE[1], image::ColorType::Rgba8).unwrap();
    }

    let key = |key| egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let start = app.scene.objects[0].position;
    let _ = ctx.run(raw(vec![key(Key::G)]), |ctx| app.frame(ctx));
    let _ = ctx.run(
        raw(vec![
            key(Key::Z),
            egui::Event::Text("0.20".into()),
            key(Key::Enter),
        ]),
        |ctx| app.frame(ctx),
    );
    let moved = app.scene.objects[0].position;
    assert!((moved.y - start.y - 0.2).abs() < 0.001);
    assert_eq!(app.undo.len(), 1);
    assert!(app.transform.is_none());
    let _ = ctx.run(raw(vec![key(Key::T), key(Key::N)]), |ctx| app.frame(ctx));
    assert!(!app.toolbar && app.sidebar);
    app.show_frame(Some(1));
    assert!(!app.playing && app.scrub == Some(1));
    assert!(app.cache[1].time < app.snapshot.time);
    let _ = ctx.run(raw(vec![key(Key::Space)]), |ctx| app.frame(ctx));
    assert!(app.playing && app.scrub.is_none());
    app.selected = Some(2);
    let _ = ctx.run(raw(vec![key(Key::Tab)]), |ctx| app.frame(ctx));
    assert!(app.edit.is_some() && !app.playing);
    assert!(app.scene.objects[1].mesh.is_some());
    let point_in_world = |object: &Object| {
        object.position + object.rotation * (object.mesh.as_ref().unwrap().points[0] * object.half)
    };
    let before = point_in_world(&app.scene.objects[1]);
    let _ = ctx.run(raw(vec![key(Key::A), key(Key::G)]), |ctx| app.frame(ctx));
    let _ = ctx.run(
        raw(vec![
            key(Key::Z),
            egui::Event::Text("0.10".into()),
            key(Key::Enter),
        ]),
        |ctx| app.frame(ctx),
    );
    let offset = point_in_world(&app.scene.objects[1]) - before;
    assert!(offset.distance(Vec3::Y * 0.10) < 0.001, "{offset}");
    let _ = ctx.run(raw(vec![key(Key::Tab)]), |ctx| app.frame(ctx));
    assert!(app.edit.is_none());

    app.maximized = true;
    app.sidebar = false;
    app.set_preset(Preset::Pour);
    app.selected = Some(2);
    let _ = ctx.run(raw(vec![key(Key::Tab)]), |ctx| app.frame(ctx));
    assert!(app.edit.is_some());
    let ramp = &app.scene.objects[1];
    let face_point = ramp.position + ramp.rotation * (Vec3::Y * ramp.half);
    let at = crate::input::project(app.camera, screen_rect.unwrap(), face_point).unwrap();
    let click = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let _ = ctx.run(
        raw(vec![egui::Event::PointerMoved(at), click(true)]),
        |ctx| app.frame(ctx),
    );
    let _ = ctx.run(raw(vec![click(false)]), |ctx| app.frame(ctx));
    let _ = ctx.run(raw(vec![key(Key::E)]), |ctx| app.frame(ctx));
    let before = app.undo.len();
    let _ = ctx.run(raw(vec![egui::Event::Text("0.15".into())]), |ctx| {
        app.frame(ctx)
    });
    assert_eq!(app.scene.objects[1].mesh.as_ref().unwrap().points.len(), 8);
    let _ = ctx.run(raw(vec![key(Key::Enter)]), |ctx| app.frame(ctx));
    assert_eq!(app.scene.objects[1].mesh.as_ref().unwrap().points.len(), 12);
    assert_eq!(app.undo.len(), before + 1);

    app.set_preset(Preset::Station);
    let _ = ctx.run(raw(vec![key(Key::F3)]), |ctx| app.frame(ctx));
    assert!(app.search);
    let _ = ctx.run(raw(vec![egui::Event::Text("tip live".into())]), |ctx| {
        app.frame(ctx)
    });
    let _ = ctx.run(raw(vec![key(Key::Enter)]), |ctx| app.frame(ctx));
    assert_eq!(
        app.live_tool.map(|tool| matches!(tool, Operation::Rotate)),
        Some(true)
    );
    let cup = app.scene.objects[1].clone();
    let pick_point = crate::input::project(
        app.camera,
        screen_rect.unwrap(),
        cup.position - Vec3::Y * 0.1,
    )
    .unwrap();
    let pointer = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let tip = pick_point + egui::vec2(100.0, 0.0);
    let _ = ctx.run(
        raw(vec![
            egui::Event::PointerMoved(pick_point),
            pointer(pick_point, true),
        ]),
        |ctx| app.frame(ctx),
    );
    let _ = ctx.run(raw(vec![egui::Event::PointerMoved(tip)]), |ctx| {
        app.frame(ctx)
    });
    assert!(app.playing && app.scene.objects[1].rotation.angle_between(cup.rotation) > 0.05);
    let _ = ctx.run(raw(vec![pointer(tip, false)]), |ctx| app.frame(ctx));
    app.reset();
    assert!(app.scene.objects[1].rotation.angle_between(cup.rotation) < 0.001);
}

fn read_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &wgpu::Texture,
    size: [u32; 2],
) -> Vec<u8> {
    let row = size[0] * 4;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * size[1]),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        image.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(size[1]),
            },
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(20)),
        })
        .unwrap();
    receive
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    let result = buffer.slice(..).get_mapped_range().to_vec();
    buffer.unmap();
    result
}

fn finish<T>(future: impl std::future::Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    let start = Instant::now();
    loop {
        if let std::task::Poll::Ready(result) = future.as_mut().poll(&mut context) {
            return result;
        }
        assert!(start.elapsed() < Duration::from_secs(25));
        std::thread::sleep(Duration::from_millis(2));
    }
}
