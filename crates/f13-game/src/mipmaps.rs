//! Mipmaps for the exported textures. The exporter writes PNGs with only the top level, so distant foliage and
//! terrain shimmer and look speckled. Once a PNG loads, a background task builds its mip chain (2x2 box filter,
//! averaged in linear light for sRGB textures) and swaps it in. Bevy's GPU mip generator can't write sRGB formats.

use bevy::asset::AssetEvent;
use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};

pub(crate) struct MipmapsPlugin;

impl Plugin for MipmapsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pending>().add_systems(Update, (start_mips, finish_mips));
    }
}

/// Mip builds in flight (screenshots wait for them).
#[derive(Resource, Default)]
pub(crate) struct Pending(Vec<(AssetId<Image>, Task<Option<Vec<u8>>>, u32)>);

/// Starts a mip build for every newly loaded RGBA8 image from a file (in-memory images, e.g. landscape weights,
/// have no path and are left alone).
fn start_mips(mut events: MessageReader<AssetEvent<Image>>, images: Res<Assets<Image>>, assets: Res<AssetServer>, mut pending: ResMut<Pending>) {
    let pool = AsyncComputeTaskPool::get();
    for e in events.read() {
        let AssetEvent::LoadedWithDependencies { id } = e else { continue };
        if assets.get_path(*id).is_none() {
            continue;
        }
        let Some(img) = images.get(*id) else { continue };
        let format = img.texture_descriptor.format;
        let srgb = match format {
            TextureFormat::Rgba8UnormSrgb => true,
            TextureFormat::Rgba8Unorm => false,
            _ => continue,
        };
        let size = img.size();
        if img.texture_descriptor.mip_level_count > 1 || size.x < 2 && size.y < 2 {
            continue;
        }
        let Some(data) = img.data.clone() else { continue };
        let levels = mip_count(size);
        pending.0.push((*id, pool.spawn(async move { build_chain(&data, size, srgb) }), levels));
    }
}

fn finish_mips(mut images: ResMut<Assets<Image>>, mut pending: ResMut<Pending>) {
    pending.0.retain_mut(|(id, task, levels)| {
        let Some(result) = block_on(future::poll_once(task)) else { return true };
        if let (Some(data), Some(mut img)) = (result, images.get_mut(*id)) {
            img.data = Some(data);
            img.texture_descriptor.mip_level_count = *levels;
            // Trilinear plus anisotropy (terrain and floors seen at grazing angles).
            let mut desc = match &img.sampler {
                ImageSampler::Descriptor(d) => d.clone(),
                ImageSampler::Default => ImageSamplerDescriptor::linear(),
            };
            desc.min_filter = ImageFilterMode::Linear;
            desc.mag_filter = ImageFilterMode::Linear;
            desc.mipmap_filter = ImageFilterMode::Linear;
            desc.anisotropy_clamp = 8;
            img.sampler = ImageSampler::Descriptor(desc);
        }
        false
    });
}

impl Pending {
    pub(crate) fn busy(&self) -> bool {
        !self.0.is_empty()
    }
}

fn mip_count(size: UVec2) -> u32 {
    32 - size.x.max(size.y).leading_zeros()
}

/// The full chain, level 0 first (level 0 is the input). None if the data doesn't match the size.
fn build_chain(data: &[u8], size: UVec2, srgb: bool) -> Option<Vec<u8>> {
    if data.len() != (size.x * size.y * 4) as usize {
        return None;
    }
    let to_linear: Vec<f32> = (0..256).map(|v| if srgb { srgb_to_linear(v as f32 / 255.0) } else { v as f32 / 255.0 }).collect();
    let mut out = data.to_vec();
    let mut prev = data.to_vec();
    let (mut w, mut h) = (size.x as usize, size.y as usize);
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                let xs = [(2 * x).min(w - 1), (2 * x + 1).min(w - 1)];
                let ys = [(2 * y).min(h - 1), (2 * y + 1).min(h - 1)];
                for c in 0..4 {
                    let mut sum = 0.0;
                    for sy in ys {
                        for sx in xs {
                            let v = prev[(sy * w + sx) * 4 + c];
                            // Alpha is always linear.
                            sum += if c == 3 { v as f32 / 255.0 } else { to_linear[v as usize] };
                        }
                    }
                    let avg = sum / 4.0;
                    let v = if c == 3 || !srgb { avg } else { linear_to_srgb(avg) };
                    next[(y * nw + x) * 4 + c] = (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                }
            }
        }
        out.extend_from_slice(&next);
        prev = next;
        (w, h) = (nw, nh);
    }
    Some(out)
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_sizes_and_values() {
        // 4x2 white/black stripes -> 2x1 -> 1x1.
        let mut data = Vec::new();
        for i in 0..8 {
            let v = if i % 2 == 0 { 255 } else { 0 };
            data.extend_from_slice(&[v, v, v, 255]);
        }
        let size = UVec2::new(4, 2);
        assert_eq!(mip_count(size), 3);
        let linear = build_chain(&data, size, false).unwrap();
        assert_eq!(linear.len(), (8 + 2 + 1) * 4);
        assert_eq!(linear[32], 128); // half grey
        assert_eq!(linear[35], 255); // alpha kept
        // In sRGB, half the light is ~188, not 128.
        let srgb = build_chain(&data, size, true).unwrap();
        assert_eq!(srgb[32], 188);
    }
}
