//! Blood on a counselor's clothes where she's hit (SCImpactEffect's "Hit Counselor Bone: %s. Trying to apply blood",
//! FUN_14035e130 on): the hit bone's body part (FUN_1403aea60: the bone a child of head, upperarm_r, upperarm_l,
//! calf_r, calf_l or spine_01, by that order; else one under the pelvis is the torso's), its gore record
//! (FUN_140341170: its CombatDamageGoreInfo's mask for the damage's weapon class, else its LimbUVTexture; its
//! TextureParameterName and ParentMaterialElementIndices), kept (FUN_1403a5620) and applied (FUN_1403aa590): the mask
//! as that texture parameter on the body's listed material elements and on all of the outfit's. The clothes' master
//! (ClothesMultiGore) shows it: shaders/clothes.wgsl.
//!
//! Here: [`Bloodied`] holds her parts' bits (the server's, replicated in NetCounselor); her clothes' damage texture is
//! their masks combined by channel (as the shader sums them), and her clothes get their own materials with it.
//! PLACEHOLDERS: the hit bone (the bone nearest the hit, no per-bone hit test), and the weapon's masks (the
//! counselors' CombatDamageGoreInfo are empty: LimbUVTexture always).

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;

/// Her body parts' names, by bit.
pub const PARTS: [&str; 6] = ["Face", "Torso", "LeftArm", "RightArm", "LeftLeg", "RightLeg"];
/// The combined damage texture's size.
const SIZE: u32 = 1024;

/// The body parts bloodied (bits of [`PARTS`]).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bloodied(pub u8);

/// A hit that bloodies her where it struck (the server's).
#[derive(Event, Clone, Copy, Debug)]
pub struct ApplyBlood {
    pub counselor: Entity,
    pub at: Vec3,
}

pub struct BloodPlugin;

impl Plugin for BloodPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(apply_blood).add_systems(Update, show_blood);
    }
}

/// The part a bone is in (FUN_1403aea60's order), by its own and its ancestors' names.
pub fn part_of_bone(chain: &[&str]) -> Option<usize> {
    let has = |n: &str| chain.iter().any(|b| b.eq_ignore_ascii_case(n));
    if has("head") {
        Some(0)
    } else if has("upperarm_r") {
        Some(3)
    } else if has("upperarm_l") {
        Some(2)
    } else if has("calf_r") {
        Some(5)
    } else if has("calf_l") {
        Some(4)
    } else if has("spine_01") || has("pelvis") {
        Some(1)
    } else {
        None
    }
}

#[allow(clippy::type_complexity)]
fn apply_blood(
    ev: On<ApplyBlood>,
    mut commands: Commands,
    children: Query<&Children>,
    parents: Query<&ChildOf>,
    skins: Query<&bevy::mesh::skinning::SkinnedMesh>,
    transforms: Query<&GlobalTransform>,
    names: Query<&Name>,
    bloodied: Query<&Bloodied>,
) {
    // The bone nearest the hit, among her skeleton's.
    let Some((joint, _)) = children
        .iter_descendants(ev.counselor)
        .filter_map(|e| skins.get(e).ok())
        .flat_map(|s| s.joints.iter().copied())
        .filter_map(|j| Some((j, transforms.get(j).ok()?.translation().distance(ev.at))))
        .min_by(|a, b| a.1.total_cmp(&b.1))
    else {
        return;
    };
    let chain: Vec<&str> = std::iter::once(joint).chain(parents.iter_ancestors(joint)).filter_map(|e| names.get(e).ok().map(|n| n.as_str())).collect();
    let Some(part) = part_of_bone(&chain) else { return };
    let old = bloodied.get(ev.counselor).map_or(0, |b| b.0);
    let new = old | 1 << part;
    info!("blood: hit at {} (bone {}): {}", ev.at, chain.first().unwrap_or(&"?"), PARTS[part]);
    if new != old {
        commands.entity(ev.counselor).insert(Bloodied(new));
    }
}

/// Her clothes' damage texture from her parts' masks (r: their blue, g: their alpha, b: their red, a: 0 — no
/// Environment Damage), and her clothes on their own materials with it.
#[allow(clippy::too_many_arguments)]
fn show_blood(
    changed: Query<(Entity, &Bloodied), Changed<Bloodied>>,
    children: Query<&Children>,
    mut clothes: Query<&mut MeshMaterial3d<crate::materials::ClothesMaterial>>,
    mut materials: ResMut<Assets<crate::materials::ClothesMaterial>>,
    mut images: ResMut<Assets<Image>>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
    export: Res<crate::ExportRoot>,
    mut masks: Local<HashMap<String, Option<Vec<u8>>>>,
) {
    let Some(counselor) = counselor else { return };
    for (e, b) in &changed {
        let mut sum = vec![0u16; (SIZE * SIZE * 3) as usize];
        for (i, name) in PARTS.iter().enumerate() {
            if b.0 & (1 << i) == 0 {
                continue;
            }
            let Some(info) = counselor.info.body_parts.get(*name) else { continue };
            // (The clothes' parameters: the head's is the face's own.)
            if !info.parameter.as_deref().is_some_and(|p| p != "Head Damage") {
                continue;
            }
            let Some(path) = &info.mask else { continue };
            let Some(px) = masks.entry(path.clone()).or_insert_with(|| load_mask(&export.0.join(path))) else { continue };
            for (k, s) in px.chunks_exact(4).enumerate() {
                sum[k * 3] += s[2] as u16;
                sum[k * 3 + 1] += s[3] as u16;
                sum[k * 3 + 2] += s[0] as u16;
            }
        }
        let data: Vec<u8> = sum.chunks_exact(3).flat_map(|c| [c[0].min(255) as u8, c[1].min(255) as u8, c[2].min(255) as u8, 0]).collect();
        let mut image = Image::new(Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 }, TextureDimension::D2, data, TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
        image.sampler = ImageSampler::linear();
        let damage = images.add(image);
        info!("blood: {} on {:?}", PARTS.iter().enumerate().filter(|(i, _)| b.0 & (1 << i) != 0).map(|(_, n)| *n).collect::<Vec<_>>().join(", "), e);
        for c in children.iter_descendants(e) {
            let Ok(mut m) = clothes.get_mut(c) else { continue };
            let Some(mut own) = materials.get(&m.0).cloned() else { continue };
            own.extension.damage = damage.clone();
            m.0 = materials.add(own);
        }
    }
}

/// A mask's pixels (RGBA8) at the combined texture's size (nearest).
fn load_mask(path: &std::path::Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    let image = Image::from_buffer(&bytes, ImageType::Extension("png"), CompressedImageFormats::NONE, false, ImageSampler::Default, RenderAssetUsages::RENDER_WORLD).ok()?;
    let image = if image.texture_descriptor.format == TextureFormat::Rgba8Unorm { image } else { image.convert(TextureFormat::Rgba8Unorm)? };
    let (w, h) = (image.width(), image.height());
    let data = image.data?;
    let mut out = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (sx, sy) = (x * w / SIZE, y * h / SIZE);
            let i = ((sy * w + sx) * 4) as usize;
            out.extend_from_slice(&data[i..i + 4]);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bone_is_in_its_part() {
        assert_eq!(part_of_bone(&["hand_r", "lowerarm_r", "upperarm_r", "clavicle_r", "spine_03"]), Some(3));
        assert_eq!(part_of_bone(&["head", "neck_01"]), Some(0));
        assert_eq!(part_of_bone(&["thigh_l", "pelvis"]), Some(1));
        assert_eq!(part_of_bone(&["foot_l", "calf_l", "thigh_l", "pelvis"]), Some(4));
        assert_eq!(part_of_bone(&["root"]), None);
    }
}
