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
//!   the part's LimbUVTexture) on the part's parameter, then the part's DetachLimb (0x14033d600): her body hides
//!   BoneToHide (else the part's attach socket: SCCounselorCharacter's constructor attaches Face to head, Torso to
//!   root, the arms to upperarm_l / upperarm_r, the legs to calf_l / calf_r; HideBoneByName, PBO_None) and gets the
//!   part's ParentLimbGoreCap (FUN_14033c980: a skeletal mesh following her pose, at ParentLimbGoreCapSocket, which
//!   none sets, snapped) and ParentLimbBloodEffect (FUN_14033bb80; none is set, the native constructor sets none).
//!   With bShowLimbAfterDetach the limb's own mesh shows where it was (SetMasterPoseComponent(null) after taking the
//!   body's pose at its socket), simulates (SetAllBodiesSimulatePhysics, the Ragdoll profile) and takes the Impulse,
//!   with its LimbGoreCap (FUN_14033c760: a static mesh at LimbGoreCapSocket); every take's notify has it off.
//! - the death's dismember roll (SCCharacter's TakeDamage, FUN_1403d7410): not in a context kill, the killer holding a
//!   weapon, a point damage: rand() / RAND_MAX x 100 (at most 100) under his CombatDismemberChance (Jason_Base: 100)
//!   detaches the hit bone's part (FUN_1403aea60) if it's still on her and isn't her torso: SERVER_DetachLimb
//!   (FUN_14047a890, no impulse), MULTICAST_DetachLimb's DetachLimb (FUN_1403be460) with the limb shown.
//!
//! Here: [`Gore`] holds her records and detached parts (the server's, replicated in NetCounselor); a hidden bone is
//! shrunk to almost nothing after her pose (its children with it, and every mesh on her skeleton, as the body's slaves
//! follow its bone visibility); a gore cap's skin is bound to her bones as the outfit's is. A shown limb is its mesh
//! on a skeleton of its own, posed as hers, the physics asset's bodies from its bone down simulated (ragdoll.rs); one
//! on a skeleton of another (the face's) goes where it hung on her, with what hangs off it (her hair), and drops.
//! PLACEHOLDERS: the blow's bone (a thrown knife's: the bone nearest where it struck; a swing's: the bone nearest
//! Jason's weapon_attach_r); the roll's rand() (the match's random numbers); the face
//! falls straight down, untumbled, onto what's below (its physics asset is one sphyl: its radius rests on the ground). Her damage textures are their masks
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

/// Her gore records: (texture parameter, mask's .png path), in the order kept; her detached parts, and those of them
/// shown (dropped), as bits by [`PARTS`] index.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Gore(pub Vec<(String, String)>, pub u8, pub u8);

/// A combat kill by Jason (not a context kill): the death's dismember roll (the server's).
#[derive(Event, Clone, Copy, Debug)]
pub struct Dismember {
    pub counselor: Entity,
    /// Where the blow landed (a thrown knife's point); None: his swing (his weapon_attach_r bone).
    pub at: Option<Vec3>,
}

/// The bone Jason's weapon hangs off (his Weapon_Attach socket's).
const WEAPON_BONE: &str = "weapon_attach_r";

/// A dropped limb: its part, the rig it came off, and its skeleton (filled once its scene is in).
#[derive(Component)]
struct FallingLimb {
    part: usize,
    from: Entity,
    file: String,
    bones: HashMap<String, Entity>,
}

/// A dropped limb on a skeleton not hers, falling: its speed (m/s, down) and radius.
#[derive(Component)]
struct Dropping(f32, f32);

/// A dropped limb's scene is in.
#[derive(Component)]
struct LimbReady;

/// A dropped limb posed as her (its ragdoll is built the next frame, from the propagated pose).
#[derive(Component)]
struct LimbPosed;

/// The bone a part's DetachLimb hides when its BoneToHide is unset: its attach socket (SCCounselorCharacter's
/// constructor), by [`PARTS`] index.
const ATTACH: [&str; 6] = ["head", "root", "upperarm_l", "upperarm_r", "calf_l", "calf_r"];

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
            .add_observer(combat_dismember)
            .add_systems(Update, (kill_gore.run_if(crate::net::simulating), show_gore, gore_caps, drop_limbs, pose_limbs, limb_ragdolls, drop_falling, limb_blood).chain());
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
    let Some((part, bone)) = part_at(ev.counselor, ev.at, &children, &parents, &skins, &transforms, &names) else { return };
    let chain = [bone.as_str()];
    let Some(r) = record(&counselor, part, None, None) else { return };
    info!("blood: hit at {} (bone {}): {} {}", ev.at, chain.first().unwrap_or(&"?"), PARTS[part], r.0);
    let mut g = gore.get(ev.counselor).cloned().unwrap_or_default();
    g.0.retain(|(p, _)| *p != r.0);
    g.0.push(r);
    commands.entity(ev.counselor).insert(g);
}

/// The part of hers nearest a point: the nearest of her skeleton's bones, its part (and its name).
fn part_at(
    counselor: Entity,
    at: Vec3,
    children: &Query<&Children>,
    parents: &Query<&ChildOf>,
    skins: &Query<&bevy::mesh::skinning::SkinnedMesh>,
    transforms: &Query<&GlobalTransform>,
    names: &Query<&Name>,
) -> Option<(usize, String)> {
    let (joint, _) = children
        .iter_descendants(counselor)
        .filter_map(|e| skins.get(e).ok())
        .flat_map(|s| s.joints.iter().copied())
        .filter_map(|j| Some((j, transforms.get(j).ok()?.translation().distance(at))))
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    let chain: Vec<&str> = std::iter::once(joint).chain(parents.iter_ancestors(joint)).filter_map(|e| names.get(e).ok().map(|n| n.as_str())).collect();
    Some((part_of_bone(&chain)?, chain.first()?.to_string()))
}

/// The death's dismember roll, and the hit part detached and shown.
#[allow(clippy::too_many_arguments)]
fn combat_dismember(
    ev: On<Dismember>,
    mut commands: Commands,
    (children, parents, skins, transforms, names): (Query<&Children>, Query<&ChildOf>, Query<&bevy::mesh::skinning::SkinnedMesh>, Query<&GlobalTransform>, Query<&Name>),
    rigs: Query<&crate::character::Rig>,
    gore: Query<&Gore>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
    killer: Option<Res<crate::character::KillerCharacter>>,
    mut rng: ResMut<crate::GameRng>,
) {
    let (Some(counselor), Some(killer)) = (counselor, killer) else { return };
    let chance = killer.info.values.get("CombatDismemberChance").copied().unwrap_or(0.0) as i32;
    let roll = ((rng.0.frand() * 100.0) as i32).min(100);
    if roll >= chance {
        return;
    }
    let weapon = || {
        let rig = rigs.iter().find(|r| matches!(r.model, crate::character::Model::Killer))?;
        Some(transforms.get(*rig.bones.get(WEAPON_BONE)?).ok()?.translation())
    };
    let Some(at) = ev.at.or_else(weapon) else { return };
    let Some((part, bone)) = part_at(ev.counselor, at, &children, &parents, &skins, &transforms, &names) else { return };
    let mut g = gore.get(ev.counselor).cloned().unwrap_or_default();
    if part == 1 || g.1 & (1 << part) != 0 {
        return;
    }
    info!("blood: combat dismember (roll {roll} < {chance}): {} (bone {bone})", PARTS[part]);
    if let Some(r) = record(&counselor, part, None, None) {
        g.0.retain(|(p, _)| *p != r.0);
        g.0.push(r);
    }
    g.1 |= 1 << part;
    g.2 |= 1 << part;
    commands.entity(ev.counselor).insert(g);
}

/// A shown part's limb, where it was on her.
fn drop_limbs(
    mut commands: Commands,
    assets: Res<AssetServer>,
    rigs: Query<(Entity, &ChildOf, &crate::character::Rig)>,
    gore: Query<&Gore>,
    globals: Query<&GlobalTransform>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
    mut dropped: Local<HashMap<Entity, u8>>,
) {
    let Some(counselor) = counselor else { return };
    for (rig_entity, parent, rig) in &rigs {
        let Ok(g) = gore.get(parent.parent()) else { continue };
        if !matches!(rig.model, crate::character::Model::Counselor) || rig.bones.is_empty() {
            continue;
        }
        let done = dropped.entry(rig_entity).or_default();
        let new = g.2 & !*done;
        *done |= new;
        let Ok(at) = globals.get(rig_entity) else { continue };
        for part in (0..PARTS.len()).filter(|i| new & (1 << i) != 0) {
            let Some(file) = counselor.info.body_parts.get(PARTS[part]).and_then(|i| i.limb.clone()) else { continue };
            info!("blood: {} dropped ({})", PARTS[part], file.rsplit('/').next().unwrap_or(""));
            let ready = |ev: On<bevy::world_serialization::WorldInstanceReady>, mut commands: Commands| {
                commands.entity(ev.entity).insert(LimbReady);
            };
            commands
                .spawn((
                    at.compute_transform(),
                    Visibility::default(),
                    bevy::world_serialization::WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(file.clone()))),
                    FallingLimb { part, from: rig_entity, file, bones: HashMap::new() },
                ))
                .observe(ready);
        }
    }
}

/// A dropped limb's skeleton takes her pose (its own bone at full size); one not on her skeleton goes where it hung
/// on her, with the parts hanging off it.
#[allow(clippy::too_many_arguments)]
fn pose_limbs(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut limbs: Query<(Entity, &mut FallingLimb), (With<LimbReady>, Without<LimbPosed>)>,
    rigs: Query<&crate::character::Rig>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut transforms: Query<&mut Transform>,
    globals: Query<&GlobalTransform>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
) {
    let Some(counselor) = counselor else { return };
    for (e, mut limb) in &mut limbs {
        let Ok(rig) = rigs.get(limb.from) else {
            commands.entity(e).despawn();
            continue;
        };
        limb.bones = children
            .iter_descendants(e)
            .filter_map(|c| names.get(c).ok().filter(|n| counselor.info.bones.iter().any(|b| b.name == n.as_str())).map(|n| (n.to_string(), c)))
            .collect();
        for (name, &b) in &limb.bones {
            let Some(t) = rig.bones.get(name).and_then(|&src| transforms.get(src).ok()).copied() else { continue };
            if let Ok(mut d) = transforms.get_mut(b) {
                *d = t;
            }
        }
        let bone = hidden_bone(&counselor, limb.part);
        if let Some(mut d) = limb.bones.get(bone).and_then(|&b| transforms.get_mut(b).ok()) {
            d.scale = Vec3::ONE;
        } else if let Some(p) = counselor.info.parts.iter().find(|p| p.file == limb.file) {
            // Where it hung (its attach bone's place on her, at her own scale: that bone is hidden now).
            let (Some(at), Ok(rig_at)) = (rig.bones.get(&p.attach).and_then(|&b| globals.get(b).ok()), globals.get(limb.from)) else { continue };
            let mut at = at.compute_transform();
            at.scale = rig_at.compute_transform().scale;
            let xf = at * p.transform.as_ref().map(crate::to_transform).unwrap_or_default();
            if let Ok(mut d) = transforms.get_mut(e) {
                *d = xf;
            }
            for c in counselor.info.parts.iter().filter(|c| c.attach == p.name && !c.file.is_empty()) {
                let cx = c.transform.as_ref().map(crate::to_transform).unwrap_or_default();
                commands.spawn((cx, Visibility::default(), bevy::world_serialization::WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(c.file.clone()))), ChildOf(e)));
            }
            // Its own physics asset's body (the face's: one sphyl on its Face bone), else her body's at that bone.
            let info = counselor.info.body_parts.get(PARTS[limb.part]);
            let radius = info.and_then(|i| i.limb_bodies.first()).or_else(|| counselor.info.ragdoll.iter().find(|b| b.bone == p.attach)).map_or(0.1, |b| b.radius / 100.0);
            commands.entity(e).insert(Dropping(0.0, radius));
        }
        commands.entity(e).insert(LimbPosed);
    }
}

/// A posed limb's ragdoll (its bodies from its bone down) and its gore cap.
fn limb_ragdolls(
    mut commands: Commands,
    assets: Res<AssetServer>,
    limbs: Query<(Entity, &FallingLimb), (With<LimbPosed>, Without<LimbCapped>)>,
    transforms: Query<&Transform>,
    globals: Query<&GlobalTransform>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
) {
    let Some(counselor) = counselor else { return };
    for (e, limb) in &limbs {
        let bone = hidden_bone(&counselor, limb.part);
        commands.entity(e).insert(LimbCapped);
        if limb.bones.contains_key(bone) {
            match crate::ragdoll::limb_ragdoll(&counselor.info, &limb.bones, bone, &transforms, &globals) {
                Some(r) => {
                    info!("blood: the dropped {}'s ragdoll: {} bodies", PARTS[limb.part], r.bodies());
                    commands.entity(e).insert(r);
                }
                None => warn!("blood: no ragdoll for the dropped {}", PARTS[limb.part]),
            }
        }
        let info = counselor.info.body_parts.get(PARTS[limb.part]);
        if let Some(cap) = info.and_then(|i| i.limb_cap.clone()) {
            let at = info.and_then(|i| i.limb_cap_at.as_ref());
            let parent = at.and_then(|s| limb.bones.get(&s.attach)).or_else(|| limb.bones.get(bone)).copied().unwrap_or(e);
            let xf = at.and_then(|s| s.transform.as_ref()).map(crate::to_transform).unwrap_or_default();
            commands.spawn((xf, Visibility::default(), bevy::world_serialization::WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(cap))), ChildOf(parent)));
        }
    }
}

/// Her damage textures (her clothes', her skin's) as last combined: a dropped limb takes them (its materials are the
/// body's, which the game's gore records set on every element).
#[derive(Component, Clone)]
struct GoreTextures(Handle<Image>, Handle<Image>);

/// A dropped limb's clothes and skin with her damage textures (once its materials are in).
#[allow(clippy::type_complexity)]
fn limb_blood(
    limbs: Query<(Entity, &FallingLimb)>,
    parents: Query<&ChildOf>,
    textures: Query<&GoreTextures>,
    children: Query<&Children>,
    (mut clothes, mut skin): (Query<&mut MeshMaterial3d<crate::materials::ClothesMaterial>>, Query<&mut MeshMaterial3d<crate::materials::SkinMaterial>>),
    (mut clothes_materials, mut skin_materials): (ResMut<Assets<crate::materials::ClothesMaterial>>, ResMut<Assets<crate::materials::SkinMaterial>>),
) {
    for (e, limb) in &limbs {
        let Some(t) = parents.get(limb.from).ok().and_then(|p| textures.get(p.parent()).ok()) else { continue };
        for c in children.iter_descendants(e) {
            if let Ok(mut m) = clothes.get_mut(c) {
                if let Some(mut own) = clothes_materials.get(&m.0).filter(|o| o.extension.damage != t.0).cloned() {
                    own.extension.damage = t.0.clone();
                    m.0 = clothes_materials.add(own);
                }
            }
            if let Ok(mut m) = skin.get_mut(c) {
                if let Some(mut own) = skin_materials.get(&m.0).filter(|o| o.extension.damage != t.1).cloned() {
                    own.extension.damage = t.1.clone();
                    m.0 = skin_materials.add(own);
                }
            }
        }
    }
}

/// A dropped limb has its ragdoll (if any) and gore cap.
#[derive(Component)]
struct LimbCapped;

/// A dropped limb not on her skeleton falls (UE's gravity, 980 cm/s^2) until it rests on what's below.
fn drop_falling(
    mut commands: Commands,
    time: Res<Time>,
    collision: Res<crate::collision::WorldCollision>,
    dynamic: Res<crate::collision::DynamicColliders>,
    mut limbs: Query<(Entity, &mut Transform, &mut Dropping)>,
) {
    let world = collision.with(&dynamic);
    let dt = time.delta_secs();
    for (e, mut xf, mut d) in &mut limbs {
        d.0 += 9.8 * dt;
        let p = xf.translation;
        let next = p.y - d.0 * dt;
        let ground = f13_core::collision::surface_below(&world, p.x, p.z, p.y + 0.5, next - 1.0).map(|g| g.0);
        match ground.filter(|&g| next - d.1 <= g) {
            Some(g) => {
                xf.translation.y = g + d.1;
                info!("blood: a dropped limb at rest at {:?}", xf.translation);
                commands.entity(e).remove::<Dropping>();
            }
            None => xf.translation.y = next,
        }
    }
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
        let mut g = gore.get(owner).cloned().unwrap_or_default();
        let before = g.clone();
        for n in list.iter().filter(|n| n.time > from && n.time <= *t) {
            let part = part_of_limb(&n.limb);
            if n.kind == "Detach" {
                info!("blood: {take} at {:.2}s: {} detached", n.time, PARTS[part]);
                g.1 |= 1 << part;
            }
            let parameter = if n.kind == "Blood" { n.parameter.as_deref() } else { None };
            let Some(r) = record(&counselor, part, n.mask.as_deref(), parameter) else { continue };
            info!("blood: {take} at {:.2}s: {} {} ({})", n.time, n.kind, r.0, r.1.rsplit('/').next().unwrap_or(""));
            g.0.retain(|(p, _)| *p != r.0);
            g.0.push(r);
        }
        if g != before {
            commands.entity(owner).insert(g);
        }
    }
}

/// A hidden bone's scale. UE hides it in the skinning only (BoneVisibilityStates), its pose (and the ragdoll's) left
/// alone; here the bone itself shrinks, by this much rather than to nothing, so its children's global transforms
/// still decompose (the ragdoll reads them). PLACEHOLDER: a skin-only hide.
const HIDDEN: f32 = 1e-3;

/// The bone a detached part hides.
fn hidden_bone(counselor: &crate::character::CounselorCharacter, part: usize) -> &str {
    counselor.info.body_parts.get(PARTS[part]).and_then(|i| i.bone_to_hide.as_deref()).filter(|b| *b != "None").unwrap_or(ATTACH[part])
}

/// Her detached parts' bones scaled to nothing, after her pose (character.rs's post-animation chain).
pub(crate) fn hide_detached(rigs: Query<(&ChildOf, &crate::character::Rig)>, gore: Query<&Gore>, counselor: Option<Res<crate::character::CounselorCharacter>>, mut transforms: Query<&mut Transform>) {
    let Some(counselor) = counselor else { return };
    for (parent, rig) in &rigs {
        let Ok(g) = gore.get(parent.parent()) else { continue };
        if g.1 == 0 || !matches!(rig.model, crate::character::Model::Counselor) {
            continue;
        }
        for part in (0..PARTS.len()).filter(|i| g.1 & (1 << i) != 0) {
            if let Some(mut xf) = rig.bones.get(hidden_bone(&counselor, part)).and_then(|&b| transforms.get_mut(b).ok()) {
                xf.scale = Vec3::splat(HIDDEN);
            }
        }
    }
}

/// A detached part's gore cap on her (its ParentLimbGoreCap, following her pose).
fn gore_caps(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut rigs: Query<(Entity, &ChildOf, &mut crate::character::Rig)>,
    gore: Query<&Gore>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
    mut capped: Local<HashMap<Entity, u8>>,
) {
    let Some(counselor) = counselor else { return };
    for (rig_entity, parent, mut rig) in &mut rigs {
        let Ok(g) = gore.get(parent.parent()) else { continue };
        let done = capped.entry(rig_entity).or_default();
        let new = g.1 & !*done;
        for part in (0..PARTS.len()).filter(|i| new & (1 << i) != 0) {
            *done |= 1 << part;
            if let Some(file) = counselor.info.body_parts.get(PARTS[part]).and_then(|i| i.gore_cap.clone()) {
                info!("blood: {} gore cap {}", PARTS[part], file.rsplit('/').next().unwrap_or(""));
                rig.follow(&mut commands, &assets, rig_entity, file);
            }
        }
    }
}

/// Her damage textures from her records' masks (r: their blue, g: their alpha, b: their red, a: 0 — no Environment
/// or Burn Damage): her clothes' (their parameters: not Head Damage) and her skin's (all of them), and her clothes
/// and skin on their own materials with them.
#[allow(clippy::too_many_arguments)]
fn show_gore(
    mut commands: Commands,
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
        commands.entity(e).insert(GoreTextures(on_clothes.clone(), on_skin.clone()));
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
    fn a_detached_part_hides_its_socket() {
        assert_eq!(ATTACH[part_of_limb("None")], "head");
        assert_eq!(ATTACH[part_of_limb("ELimb::RIGHT_ARM")], "upperarm_r");
        assert_eq!(ATTACH[part_of_limb("ELimb::LEFT_ARM")], "upperarm_l");
    }

    #[test]
    fn a_limb_is_its_part() {
        assert_eq!(part_of_limb("ELimb::LEFT_ARM"), 2);
        assert_eq!(part_of_limb("None"), 0);
        assert_eq!(part_of_limb("ELimb::RIGHT_LEG"), 5);
    }
}
