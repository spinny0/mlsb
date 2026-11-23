use std::{
    io::{BufRead, Cursor, Seek},
    sync::mpsc::{self, Receiver},
};

use crate::scene::{EnvironmentMode, Lighting};
use eframe::egui_wgpu::wgpu;
use half::f16;

#[derive(Clone, PartialEq, Eq)]
struct Key(EnvironmentMode, String, u32);

fn key(l: &Lighting) -> Key {
    Key(
        l.environment,
        if l.environment == EnvironmentMode::File {
            l.panorama.trim().into()
        } else {
            String::new()
        },
        if l.environment == EnvironmentMode::Garden {
            l.environment_width.min(2048)
        } else {
            l.environment_width
        },
    )
}

struct Panorama {
    width: u32,
    levels: Vec<Vec<[u16; 4]>>,
}

fn downsample(source: &image::Rgb32FImage, width: u32, height: u32) -> image::Rgb32FImage {
    let sx = source.width() as f32 / width as f32;
    let sy = source.height() as f32 / height as f32;
    let area = sx * sy;
    image::Rgb32FImage::from_fn(width, height, |x, y| {
        let (left, right) = (x as f32 * sx, (x + 1) as f32 * sx);
        let (top, bottom) = (y as f32 * sy, (y + 1) as f32 * sy);
        let mut color = [0.0; 3];
        for row in top.floor() as u32..(bottom.ceil() as u32).min(source.height()) {
            let wy = bottom.min(row as f32 + 1.0) - top.max(row as f32);
            for column in left.floor() as u32..(right.ceil() as u32).min(source.width()) {
                let wx = right.min(column as f32 + 1.0) - left.max(column as f32);
                let weight = wx * wy / area;
                for (channel, &value) in color.iter_mut().zip(&source.get_pixel(column, row).0) {
                    *channel += value.clamp(0.0, 65504.0) * weight;
                }
            }
        }
        image::Rgb(color)
    })
}

fn decode_reader<R: BufRead + Seek>(
    mut reader: image::ImageReader<R>,
    requested: u32,
) -> Result<Panorama, String> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let source = reader.decode().map_err(|e| e.to_string())?.into_rgb32f();
    if source.width() > 8192 || source.height() > 4096 {
        return Err("Panoramas can be up to 8192 × 4096.".into());
    }
    if source.width() != source.height() * 2 {
        return Err("Choose a 2:1 latitude-longitude panorama.".into());
    }
    if source.pixels().any(|p| p.0.iter().any(|v| !v.is_finite())) {
        return Err("Invalid panorama radiance.".into());
    }
    let limit = requested.min(source.width()).max(2);
    let width = 1 << limit.ilog2();
    let mut current = if source.width() == width {
        source
    } else {
        downsample(&source, width, width / 2)
    };
    let mut levels = Vec::new();
    loop {
        levels.push(
            current
                .pixels()
                .map(|p| {
                    let [r, g, b] = p.0.map(|v| f16::from_f32(v.clamp(0.0, 65504.0)).to_bits());
                    [r, g, b, f16::ONE.to_bits()]
                })
                .collect(),
        );
        if current.width() == 1 && current.height() == 1 {
            break;
        }
        current = downsample(
            &current,
            (current.width() / 2).max(1),
            (current.height() / 2).max(1),
        );
    }
    Ok(Panorama { width, levels })
}

fn decode(key: &Key) -> Result<Panorama, String> {
    match key.0 {
        EnvironmentMode::Studio => Err("Choose a panorama.".into()),
        EnvironmentMode::Garden => decode_reader(
            image::ImageReader::with_format(
                Cursor::new(include_bytes!("../../assets/garden-2k.hdr")),
                image::ImageFormat::Hdr,
            ),
            key.2,
        ),
        EnvironmentMode::File => {
            let extension = std::path::Path::new(&key.1)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            let format = if extension.eq_ignore_ascii_case("hdr") {
                image::ImageFormat::Hdr
            } else if extension.eq_ignore_ascii_case("exr") {
                image::ImageFormat::OpenExr
            } else {
                return Err("Enter the path to an HDR or EXR panorama.".into());
            };
            let mut reader = image::ImageReader::open(&key.1).map_err(|e| e.to_string())?;
            reader.set_format(format);
            decode_reader(reader, key.2)
        }
    }
}

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, map: &Panorama) -> wgpu::TextureView {
    let h = (map.width / 2).max(1);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Linear HDR panorama"),
        size: wgpu::Extent3d {
            width: map.width,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: map.levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (mip, data) in map.levels.iter().enumerate() {
        let (width, height) = ((map.width >> mip).max(1), (h >> mip).max(1));
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: mip as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(data),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 8),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }
    texture.create_view(&Default::default())
}

pub struct Environment {
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub status: String,
    wanted: Option<Key>,
    installed: Option<Key>,
    attempted: Option<Key>,
    pending: Option<(Key, Receiver<Result<Panorama, String>>)>,
}

impl Environment {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let stub = Panorama {
            width: 1,
            levels: vec![vec![[0, 0, 0, f16::ONE.to_bits()]]],
        };
        Self {
            view: upload(device, queue, &stub),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::Repeat,
                min_filter: wgpu::FilterMode::Linear,
                mag_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            status: "Studio".into(),
            wanted: None,
            installed: None,
            attempted: None,
            pending: None,
        }
    }

    pub fn active(&self, light: &Lighting) -> bool {
        self.installed.as_ref() == Some(&key(light))
    }
    pub fn loading(&self) -> bool {
        self.pending.is_some()
    }

    pub fn poll(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        light: &Lighting,
    ) -> (bool, bool) {
        let wanted = key(light);
        let mut changed = self.wanted.as_ref() != Some(&wanted);
        if changed {
            self.attempted = None;
        }
        self.wanted = Some(wanted.clone());
        let mut uploaded = false;
        if let Some((_, rx)) = &self.pending
            && let Ok(result) = rx.try_recv()
        {
            let (loaded, _) = self.pending.take().unwrap();
            if loaded == wanted {
                self.attempted = Some(loaded.clone());
                match result {
                    Ok(map) => {
                        self.status = format!("{} × {} HDR", map.width, map.width / 2);
                        self.view = upload(device, queue, &map);
                        self.installed = Some(loaded);
                        uploaded = true;
                    }
                    Err(error) => self.status = error,
                }
                changed = true;
            }
        }
        if wanted.0 == EnvironmentMode::Studio {
            self.status = "Studio".into();
        } else if self.pending.is_none()
            && self.installed.as_ref() != Some(&wanted)
            && self.attempted.as_ref() != Some(&wanted)
        {
            let (tx, rx) = mpsc::channel();
            let work = wanted.clone();
            self.pending = Some((wanted, rx));
            self.status = "Loading panorama…".into();
            std::thread::spawn(move || {
                let _ = tx.send(decode(&work));
            });
        } else if self.installed.as_ref() == Some(&wanted) && self.status == "Studio" {
            self.status = "Panorama ready".into();
        }
        (changed, uploaded)
    }
}

#[cfg(test)]
pub fn check_formats() -> Result<(), String> {
    let garden = decode(&Key(EnvironmentMode::Garden, String::new(), 1024))?;
    assert_eq!(garden.width, 1024);
    assert!(
        garden.levels[0]
            .iter()
            .any(|p| f16::from_bits(p[0]).to_f32() > 1.0)
    );
    let source = image::DynamicImage::ImageRgb32F(image::Rgb32FImage::from_pixel(
        8,
        4,
        image::Rgb([1.2, 0.2, 0.4]),
    ));
    let mut bytes = Cursor::new(Vec::new());
    source
        .write_to(&mut bytes, image::ImageFormat::OpenExr)
        .map_err(|e| e.to_string())?;
    let map = decode_reader(
        image::ImageReader::with_format(
            Cursor::new(bytes.into_inner()),
            image::ImageFormat::OpenExr,
        ),
        4,
    )?;
    assert!(
        map.levels
            .iter()
            .all(|level| f16::from_bits(level[0][0]).to_f32() > 1.19)
    );
    Ok(())
}
