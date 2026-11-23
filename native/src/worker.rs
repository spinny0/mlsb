use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

use crate::{
    fluid::{Fluid, Quality},
    scene::Scene,
};

pub struct Job {
    pub epoch: u64,
    pub scene: Scene,
    pub quality: Quality,
    pub steps: u32,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub epoch: u64,
    pub particles: Arc<[[f32; 4]]>,
    pub secondary: Arc<[[f32; 4]]>,
    pub scene: Option<Scene>,
    pub time: f32,
    pub step_time: Duration,
    pub emitted: u64,
    pub drained: u64,
    pub particle_liters: f32,
    pub initial_liters: f32,
}

pub struct Worker {
    pub jobs: SyncSender<Job>,
    pub frames: Receiver<Snapshot>,
}

impl Worker {
    pub fn start() -> std::io::Result<Self> {
        let (jobs, queue) = mpsc::sync_channel::<Job>(1);
        let (results, frames) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("MLSB pressure solver".into())
            .spawn(move || {
                let mut state = None;
                let mut epoch = 0;
                while let Ok(job) = queue.recv() {
                    if state.is_none() || epoch != job.epoch {
                        state = Some(Fluid::new(&job.scene, job.quality));
                        epoch = job.epoch;
                    }
                    let fluid = state.as_mut().unwrap();
                    let start = Instant::now();
                    let steps = job.steps.min(2);
                    for _ in 0..steps {
                        fluid.step(&job.scene, job.quality);
                    }
                    let step_time = start.elapsed() / steps.max(1);
                    let frame = Snapshot {
                        epoch,
                        particles: fluid.snapshot().into(),
                        secondary: fluid.secondary_snapshot().into(),
                        scene: Some(job.scene),
                        time: fluid.time,
                        step_time,
                        emitted: fluid.emitted,
                        drained: fluid.drained,
                        particle_liters: fluid.particle_volume * 1000.0,
                        initial_liters: fluid.initial_liters,
                    };
                    if results.send(frame).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self { jobs, frames })
    }
}
