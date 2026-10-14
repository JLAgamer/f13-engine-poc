//! Dead counselors go limp: the body's physics asset (counselors/<name>.json's Ragdoll and Joints) becomes an
//! f13-core ragdoll, started from the pose she died in and simulated against the world's collision; the simulated
//! bones are posed from it and every other bone keeps its pose relative to its parent.
//!
//! The game ragdolls a counselor when she dies (SCCharacter's death sets the mesh to simulate physics). Here the
//! ragdoll starts when her health reaches zero: right away after a fatal hit, at the end of a kill animation.

use bevy::math::Affine3A;
use bevy::prelude::*;
use f13_core::ragdoll::{Body, Link, Ragdoll, RagdollSettings};
use f13_data::character::CharacterInfo;
use std::collections::HashMap;

use crate::character::{CounselorCharacter, Model, Rig};
use crate::player::Player;

pub struct RagdollPlugin;

impl Plugin for RagdollPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, simulate)
            .add_systems(Update, (start_ragdolls, pose_ragdolls).chain().after(crate::character::animate));
    }
}

/// On a rig whose counselor is dead.
#[derive(Component)]
pub struct Ragdolled {
    sim: Ragdoll,
    /// Per simulated body (as `sim.bodies`): its bone (index in the skeleton), name, parent body and scale.
    bones: Vec<(usize, String, Option<usize>, Vec3)>,
    /// Every bone's local transform when she died (the bones the ragdoll doesn't move keep it).
    rest: HashMap<usize, Transform>,
    /// The skeleton posed: its bones by name (her rig's, or a dropped limb's; blood.rs).
    bone_entities: HashMap<String, Entity>,
    /// Seconds simulated.
    pub age: f32,
}

impl Ragdolled {
    /// How many bodies it simulates.
    pub fn bodies(&self) -> usize {
        self.sim.bodies.len()
    }

    /// Where the body is (its first body, the pelvis).
    pub fn center(&self) -> Option<Vec3> {
        self.sim.bodies.first().map(|b| self.sim.particles[b.particle].pos)
    }

    /// The angle at each joint between the parent body and where the body points (degrees), for logs.
    fn angles(&self) -> String {
        let pos = |i: usize| self.sim.particles[i].pos;
        self.sim
            .bodies
            .iter()
            .zip(&self.bones)
            .filter_map(|(b, (_, name, parent, _))| {
                let pa = self.sim.bodies[(*parent)?].particle;
                Some(format!("{name} {:.0}", (pos(pa) - pos(b.particle)).angle_between(pos(b.aim) - pos(b.particle)).to_degrees()))
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn start_ragdolls(
    mut commands: Commands,
    character: Option<Res<CounselorCharacter>>,
    players: Query<&Player>,
    mut rigs: Query<(Entity, &ChildOf, &Rig, &mut AnimationPlayer), Without<Ragdolled>>,
    transforms: Query<&Transform>,
    globals: Query<&GlobalTransform>,
) {
    let Some(character) = character else { return };
    for (e, parent, rig, mut anim) in &mut rigs {
        if rig.model != Model::Counselor || rig.bones.is_empty() || players.get(parent.parent()).map_or(true, |p| p.counselor.alive()) {
            continue;
        }
        // Her take's FreezeAnimation notify passed: bIgnoreDeathRagdoll (Base_Counselor_AnimBP), she stays as it froze.
        let frozen = rig.action.as_ref().is_some_and(|(a, t)| character.info.animations.get(a.as_str()).and_then(|i| i.window("FreezeAnimation")).is_some_and(|w| *t >= w.0));
        if frozen {
            continue;
        }
        let Some(r) = build(&character.info, &rig.bones, None, &transforms, &globals) else {
            warn!("no ragdoll: the counselor export has no physics asset (re-export with tools/f13-export counselor)");
            let mut sim = Ragdoll::default();
            sim.asleep = true;
            commands.entity(e).insert(Ragdolled { sim, bones: Vec::new(), rest: HashMap::new(), bone_entities: HashMap::new(), age: 0.0 });
            continue;
        };
        info!("ragdoll: {} bodies, {} links", r.sim.bodies.len(), r.sim.links.len());
        debug!("ragdoll joints at the start: {}", r.angles());
        anim.stop_all();
        commands.entity(e).insert(r);
    }
}

fn side_name(name: &str) -> Option<(&str, bool)> {
    name.strip_suffix("_l").map(|s| (s, false)).or_else(|| name.strip_suffix("_r").map(|s| (s, true)))
}

/// A dropped limb's ragdoll (blood.rs): the physics asset's bodies from `root` down, on the limb's own skeleton
/// (`bones`), the root body free.
pub(crate) fn limb_ragdoll(info: &CharacterInfo, bones: &HashMap<String, Entity>, root: &str, transforms: &Query<&Transform>, globals: &Query<&GlobalTransform>) -> Option<Ragdolled> {
    let root = info.bones.iter().position(|b| b.name == root)?;
    build(info, bones, Some(root), transforms, globals)
}

/// Whether bone `i` is `root` or under it.
fn under(info: &CharacterInfo, mut i: usize, root: usize) -> bool {
    loop {
        if i == root {
            return true;
        }
        match info.bones.get(i).map(|b| b.parent) {
            Some(p) if p >= 0 => i = p as usize,
            _ => return false,
        }
    }
}

fn build(info: &CharacterInfo, rig_bones: &HashMap<String, Entity>, subtree: Option<usize>, transforms: &Query<&Transform>, globals: &Query<&GlobalTransform>) -> Option<Ragdolled> {
    if info.ragdoll.is_empty() {
        return None;
    }
    let index: HashMap<&str, usize> = info.bones.iter().enumerate().map(|(i, b)| (b.name.as_str(), i)).collect();
    let world = |bone: &str| rig_bones.get(bone).and_then(|&e| globals.get(e).ok()).map(|g| g.compute_transform());
    let dt = 1.0 / 64.0;
    let mut sim = Ragdoll::default();
    // One particle per body, at its bone (a limb's: only those under its root).
    let mut particle: HashMap<&str, usize> = HashMap::new();
    let mut radius: HashMap<&str, f32> = HashMap::new();
    for b in &info.ragdoll {
        if let Some(root) = subtree {
            if !index.get(b.bone.as_str()).is_some_and(|&i| under(info, i, root)) {
                continue;
            }
        }
        let Some(xf) = world(&b.bone) else { continue };
        let r = (b.radius / 100.0).max(0.03);
        particle.insert(b.bone.as_str(), sim.add(xf.translation, r, Vec3::ZERO, dt));
        radius.insert(b.bone.as_str(), r);
    }
    let children = |bone: &str| -> Vec<&str> {
        let mut c: Vec<&str> = info.joints.iter().filter(|j| j.bone2 == bone && particle.contains_key(j.bone1.as_str())).map(|j| j.bone1.as_str()).collect();
        c.sort();
        c
    };

    // Bodies: each points along its chain (the child that isn't a left/right one: spine, head; else its only child;
    // else a tip at the far end of its shape) and rolls with its left/right children (hips, shoulders).
    let mut body_of: HashMap<&str, usize> = HashMap::new();
    let mut bones = Vec::new();
    for b in &info.ragdoll {
        let (Some(&p), Some(xf), Some(&bone)) = (particle.get(b.bone.as_str()), world(&b.bone), index.get(b.bone.as_str())) else { continue };
        let kids = children(&b.bone);
        let chain = kids.iter().find(|k| side_name(k).is_none()).or(if kids.len() == 1 { kids.first() } else { None });
        let aim = match chain {
            Some(k) => particle[k],
            None => {
                let shape = b.transform.as_ref().map(crate::to_transform).unwrap_or_default();
                let mut local = shape.translation * 2.0;
                if local.length() < 0.03 {
                    // A sphyl lies along its Z (Unreal), Bevy's Y here.
                    local = shape.rotation * Vec3::Y * (b.length / 200.0 + radius[b.bone.as_str()]);
                }
                if local.length() < 0.03 {
                    p
                } else {
                    let tip = sim.add(xf.transform_point(local), radius[b.bone.as_str()] * 0.8, Vec3::ZERO, dt);
                    sim.rigid(p, tip);
                    tip
                }
            }
        };
        let left = kids.iter().find(|k| side_name(k).is_some_and(|(_, r)| !r));
        let right = kids.iter().find(|k| side_name(k).is_some_and(|(_, r)| r));
        let sides = left.zip(right).map(|(l, r)| (particle[l], particle[r]));
        let pos = |i: usize| sim.particles[i].pos;
        let body = Body {
            particle: p,
            aim,
            sides,
            rotation0: xf.rotation,
            aim0: pos(aim) - pos(p),
            side0: sides.map(|(l, r)| pos(r) - pos(l)),
        };
        body_of.insert(b.bone.as_str(), sim.bodies.len());
        sim.bodies.push(body);
        bones.push((bone, b.bone.clone(), None, xf.scale));
    }

    // Each body hangs off its parent and bends there by up to its joint's swing from the pose she died in.
    let parent_of = |bone: &str| info.joints.iter().find(|j| j.bone1 == bone).map(|j| j.bone2.as_str()).filter(|p| particle.contains_key(p));
    let swing_of = |bone: &str| info.joints.iter().find(|j| j.bone1 == bone).map_or(30.0, |j| j.swing1.max(j.swing2));
    for (i, b) in info.ragdoll.iter().filter(|b| body_of.contains_key(b.bone.as_str())).enumerate() {
        let Some(parent) = parent_of(&b.bone) else { continue };
        let (p, aim) = (sim.bodies[i].particle, sim.bodies[i].aim);
        bones[i].2 = body_of.get(parent).copied();
        sim.rigid(particle[parent], p);
        if aim != p {
            sim.bend(particle[parent], p, aim, swing_of(&b.bone));
        }
        // A left/right child (a thigh off the pelvis, an arm off the chest): its cone is also measured from the
        // parent's own parent, or the parent's chain.
        if side_name(&b.bone).is_some() && aim != p {
            let reference = parent_of(parent).map(|g| particle[g]).or_else(|| body_of.get(parent).map(|&x| sim.bodies[x].aim));
            if let Some(r) = reference.filter(|&r| r != p) {
                sim.bend(r, p, aim, swing_of(&b.bone));
            }
        }
    }
    // The torso holds its shape: left and right children rigid with each other (hips, shoulders), and the same sides
    // of the hips and shoulders nearly so (no twisting through the spine: its joints lock twist).
    let pairs: Vec<(usize, usize)> = sim.bodies.iter().filter_map(|b| b.sides).collect();
    for &(l, r) in &pairs {
        sim.rigid(l, r);
    }
    for (i, &(l1, r1)) in pairs.iter().enumerate() {
        for &(l2, r2) in &pairs[i + 1..] {
            for (a, b) in [(l1, l2), (r1, r2), (l1, r2), (r1, l2)] {
                let d = sim.particles[a].pos.distance(sim.particles[b].pos);
                sim.links.push(Link { a, b, min: d * 0.92, max: d * 1.08 });
            }
        }
    }

    let rest = info
        .bones
        .iter()
        .enumerate()
        .filter_map(|(i, b)| rig_bones.get(&b.name).and_then(|&e| transforms.get(e).ok()).map(|t| (i, *t)))
        .collect();
    Some(Ragdolled { sim, bones, rest, bone_entities: rig_bones.clone(), age: 0.0 })
}

fn simulate(
    time: Res<Time>,
    collision: Res<crate::collision::WorldCollision>,
    dynamic: Res<crate::collision::DynamicColliders>,
    mut rigs: Query<&mut Ragdolled>,
    mut settings: Local<Option<RagdollSettings>>,
) {
    let s = settings.get_or_insert_with(RagdollSettings::default);
    let world = collision.with(&dynamic);
    for mut r in &mut rigs {
        if r.sim.asleep {
            continue;
        }
        r.sim.step(&world, time.delta_secs(), s);
        let before = r.age;
        r.age += time.delta_secs();
        if (before * 2.0) as u32 != (r.age * 2.0) as u32 {
            let ground = r.center().and_then(|c| f13_core::collision::surface_below(&world, c.x, c.z, c.y + 1.0, c.y - 5.0)).map(|g| g.0);
            trace!("ragdoll t={:.1}: pelvis {:?} (ground {:?})", r.age, r.center(), ground);
        }
        if r.sim.asleep {
            let lowest = r.sim.particles.iter().map(|p| p.pos.y - p.radius).fold(f32::MAX, f32::min);
            let highest = r.sim.particles.iter().map(|p| p.pos.y).fold(f32::MIN, f32::max);
            info!("ragdoll at rest after {:.1}s: pelvis {:?}, {:.2} m from lowest to highest body", r.age, r.center(), highest - lowest);
            debug!("ragdoll joints at rest: {}", r.angles());
        }
    }
}

fn pose_ragdolls(
    character: Option<Res<CounselorCharacter>>,
    rigs: Query<&Ragdolled>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
    mut transforms: Query<&mut Transform>,
) {
    let Some(character) = character else { return };
    let info = &character.info;
    for r in &rigs {
        let mut desired: HashMap<usize, Affine3A> = HashMap::new();
        for (b, &(bone, _, _, scale)) in r.sim.bodies.iter().zip(&r.bones) {
            desired.insert(bone, Affine3A::from_scale_rotation_translation(scale, r.sim.rotation(b), r.sim.particles[b.particle].pos));
        }
        if desired.is_empty() {
            continue;
        }
        // Parents before children (the skeleton's bone order): body bones go where the ragdoll puts them, the others
        // follow their parent with the pose they died in.
        let mut world: HashMap<usize, Affine3A> = HashMap::new();
        for (i, bone) in info.bones.iter().enumerate() {
            let Some(&e) = r.bone_entities.get(&bone.name) else { continue };
            let parent_world = if bone.parent >= 0 {
                world.get(&(bone.parent as usize)).copied()
            } else {
                parents.get(e).ok().and_then(|p| globals.get(p.parent()).ok()).map(|g| g.affine())
            };
            let (Some(parent_world), Some(rest)) = (parent_world, r.rest.get(&i)) else { continue };
            match desired.get(&i) {
                Some(&d) => {
                    world.insert(i, d);
                    if let Ok(mut t) = transforms.get_mut(e) {
                        *t = Transform::from_matrix(Mat4::from(parent_world.inverse() * d));
                    }
                }
                None => {
                    world.insert(i, parent_world * rest.compute_affine());
                }
            }
        }
    }
}
