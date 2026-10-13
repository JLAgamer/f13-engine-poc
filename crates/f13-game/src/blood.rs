//! Blood on a counselor (her clothes' and skin's gore layers: ClothesMultiGore, SkinGoreMaster; shaders/clothes.wgsl,
//! shaders/skin.wgsl). The game keeps gore records (FUN_1403a5620: a mask, the texture parameter it goes to, the
//! material elements) and applies them all (FUN_1403aa590: SetTextureParameterValue on the body's listed elements and
//! every material of the outfit), so a later record on a parameter replaces the earlier. They come from:
//! - an impact effect hitting her (SCImpactEffect's "Hit Counselor Bone: %s. Trying to apply blood", FUN_14035e130
//!   on): the hit bone's body part (FUN_1403aea60: the bone a child of head, upperarm_r, upperarm_l, calf_r, calf_l or
//!   spine_01, by that order; else one under the pelvis is the torso's), its record (FUN_140341170: its
//!   CombatDamageGoreInfo's mask for the damage's weapon class, else its LimbUVTexture; its TextureParameterName);
//! - a kill take's SCApplyBloodMaskAnimNotify (0x140314890): its Limb's part, its BloodMask (else the part's
//!   LimbUVTexture), its TextureParameter (else the part's), its MaterialElementIndices (else the part's);
//! - SCDetachLimbAnimNotify (0x140314b60, SCCounselorCharacter::DetachLimb 0x1403abf00): its BloodMaskOverride (else
//!   the part's LimbUVTexture) on the part's parameter, then the limb comes off (not ported).
//!
//! Here: [`Gore`] holds her records (the server's, replicated in NetCounselor); her damage textures are their masks
//! (the latest per parameter) combined by channel, as the shaders sum them, her clothes' without Head Damage; her
//! clothes and skin get their own materials with them. PLACEHOLDERS: the hit bone (the bone nearest the hit, no
//! per-bone hit test); the elements (every clothes and skin material takes them).

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;

/// Her body parts' names, by ELimb (HEAD, TORSO, LEFT_ARM, RIGHT_ARM, LEFT_LEG, RIGHT_LEG).
pub const PARTS: [&str; 6] = ["Face", "Torso", "LeftArm", "RightArm", "LeftLeg", "RightLeg"];
/// The combined damage textures' size.
const SIZE: u32 = 1024;

/// Her gore records: (texture parameter, mask's .png path), in the order kept.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct Gore(pub Vec<(String, String)>);

/// A hit that bloodies her where it struck (the server's).
#[derive(Event, Clone, Copy, Debug)]
pub struct ApplyBlood {
    pub counselor: Entity,
    pub at: Vec3,
}

/// The kill takes' gore notifies (anim_gore.json), by take key ("counselor/<take>").
#[derive(Resource, Default)]
struct AnimGore(HashMap<String, Vec<GoreNotify>>);

#[derive(serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "PascalCase")]
struct GoreNotify {
    time: f32,
    kind: String,
    limb: String,
    #[serde(default)]
    mask: Option<String>,
    #[serde(default)]
    parameter: Option<String>,
}

pub struct BloodPlugin;

impl Plugin for BloodPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AnimGore>()
            .add_systems(Startup, load_anim_gore)
            .add_observer(apply_blood)
            .add_systems(Update, (kill_gore.run_if(crate::net::simulating), show_gore).chain());
    }
}

fn load_anim_gore(export: Res<crate::ExportRoot>, mut gore: ResMut<AnimGore>) {
    if let Some(v) = std::fs::read_to_string(export.0.join("anim_gore.json")).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        gore.0 = v;
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

/// An ELimb's part (its name in the notifies: "ELimb::TORSO"; unset, HEAD).
fn part_of_limb(limb: &str) -> usize {
    match limb.rsplit(':').next().unwrap_or("") {
        "TORSO" => 1,
        "LEFT_ARM" => 2,
        "RIGHT_ARM" => 3,
        "LEFT_LEG" => 4,
        "RIGHT_LEG" => 5,
        _ => 0,
    }
}

/// A part's record: its parameter and mask, or the given ones over them.
fn record(counselor: &crate::character::CounselorCharacter, part: usize, mask: Option<&str>, parameter: Option<&str>) -> Option<(String, String)> {
    let info = counselor.info.body_parts.get(PARTS[part]);
    let parameter = parameter.filter(|p| *p != "None").map(String::from).or_else(|| info.and_then(|i| i.parameter.clone()))?;
    let mask = mask.map(String::from).or_else(|| info.and_then(|i| i.mask.clone()))?;
    Some((parameter, mask))
}

#[allow(clippy::too_many_arguments)]
fn apply_blood(
    ev: On<ApplyBlood>,
    mut commands: Commands,
    children: Query<&Children>,
    parents: Query<&ChildOf>,
    skins: Query<&bevy::mesh::skinning::SkinnedMesh>,
    transforms: Query<&GlobalTransform>,
    names: Query<&Name>,
    gore: Query<&Gore>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
) {
    let Some(counselor) = counselor else { return };
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
    let Some(r) = record(&counselor, part, None, None) else { return };
    info!("blood: hit at {} (bone {}): {} {}", ev.at, chain.first().unwrap_or(&"?"), PARTS[part], r.0);
    let mut records = gore.get(ev.counselor).map_or_else(|_| Vec::new(), |g| g.0.clone());
    records.retain(|(p, _)| *p != r.0);
    records.push(r);
    commands.entity(ev.counselor).insert(Gore(records));
}

/// Her takes' gore notifies as they pass (the server's).
fn kill_gore(
    mut commands: Commands,
    rigs: Query<(&ChildOf, &crate::character::Rig)>,
    gore: Query<&Gore>,
    notifies: Res<AnimGore>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
    mut last: Local<HashMap<Entity, (String, f32)>>,
) {
    let Some(counselor) = counselor else { return };
    for (parent, rig) in &rigs {
        if !matches!(rig.model, crate::character::Model::Counselor) {
            continue;
        }
        let owner = parent.parent();
        let Some((take, t)) = &rig.action else {
            last.remove(&owner);
            continue;
        };
        let from = match last.get(&owner) {
            Some((k, prev)) if k == take && *prev <= *t => *prev,
            _ => -1.0,
        };
        last.insert(owner, (take.clone(), *t));
        let Some(list) = notifies.0.get(&format!("counselor/{take}")) else { continue };
        let mut records = gore.get(owner).map_or_else(|_| Vec::new(), |g| g.0.clone());
        let before = records.clone();
        for n in list.iter().filter(|n| n.time > from && n.time <= *t) {
            let part = part_of_limb(&n.limb);
            let parameter = if n.kind == "Blood" { n.parameter.as_deref() } else { None };
            let Some(r) = record(&counselor, part, n.mask.as_deref(), parameter) else { continue };
            info!("blood: {take} at {:.2}s: {} {} ({})", n.time, n.kind, r.0, r.1.rsplit('/').next().unwrap_or(""));
            records.retain(|(p, _)| *p != r.0);
            records.push(r);
        }
        if records != before {
            commands.entity(owner).insert(Gore(records));
        }
    }
}

/// Her damage textures from her records' masks (r: their blue, g: their alpha, b: their red, a: 0 — no Environment
/// or Burn Damage): her clothes' (their parameters: not Head Damage) and her skin's (all of them), and her clothes
/// and skin on their own materials with them.
#[allow(clippy::too_many_arguments)]
fn show_gore(
    changed: Query<(Entity, &Gore), Changed<Gore>>,
    children: Query<&Children>,
    (mut clothes, mut skin): (Query<&mut MeshMaterial3d<crate::materials::ClothesMaterial>>, Query<&mut MeshMaterial3d<crate::materials::SkinMaterial>>),
    (mut clothes_materials, mut skin_materials): (ResMut<Assets<crate::materials::ClothesMaterial>>, ResMut<Assets<crate::materials::SkinMaterial>>),
    mut images: ResMut<Assets<Image>>,
    export: Res<crate::ExportRoot>,
    mut masks: Local<HashMap<String, Option<Vec<u8>>>>,
) {
    for (e, g) in &changed {
        let mut combine = |head: bool| {
            let mut sum = vec![0u16; (SIZE * SIZE * 3) as usize];
            for (parameter, path) in &g.0 {
                if !head && parameter == "Head Damage" {
                    continue;
                }
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
            images.add(image)
        };
        let (on_clothes, on_skin) = (combine(false), combine(true));
        info!("blood: {} on {e:?}", g.0.iter().map(|(p, m)| format!("{p} {}", m.rsplit('/').next().unwrap_or(""))).collect::<Vec<_>>().join(", "));
        for c in children.iter_descendants(e) {
            if let Ok(mut m) = clothes.get_mut(c) {
                if let Some(mut own) = clothes_materials.get(&m.0).cloned() {
                    own.extension.damage = on_clothes.clone();
                    m.0 = clothes_materials.add(own);
                }
            }
            if let Ok(mut m) = skin.get_mut(c) {
                if let Some(mut own) = skin_materials.get(&m.0).cloned() {
                    own.extension.damage = on_skin.clone();
                    m.0 = skin_materials.add(own);
                }
            }
        }
    }
}

/// A mask's pixels (RGBA8) at the combined textures' size (nearest).
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

    #[test]
    fn a_limb_is_its_part() {
        assert_eq!(part_of_limb("ELimb::LEFT_ARM"), 2);
        assert_eq!(part_of_limb("None"), 0);
        assert_eq!(part_of_limb("ELimb::RIGHT_LEG"), 5);
    }
}
