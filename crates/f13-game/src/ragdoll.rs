//! Dead counselors go limp: the body's physics asset (counselors/<name>.json's Ragdoll and Joints) becomes an
//! f13-core ragdoll, started from the pose she died in and simulated against the world's collision; the simulated
//! bones are posed from it and every other bone keeps its pose relative to its parent.
//!
//! The game ragdolls a counselor when she dies (SCCharacter's death sets the mesh to simulate physics). Here the
//! ragdoll starts when her health reaches zero: right away after a fatal hit, at the end of a kill animation.

use bevy::math::Affine3A;
use bevy::prelude::*;
use f13_core::ragdoll::{Body, Cone, Ragdoll, RagdollSettings};
use f13_data::character::CharacterInfo;
use f13_data::level::Xform;
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
    /// Seconds simulated.
    pub age: f32,
}

impl Ragdolled {
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
    collision: Res<crate::collision::WorldCollision>,
    dynamic: Res<crate::collision::DynamicColliders>,
) {
    let Some(character) = character else { return };
    for (e, parent, rig, mut anim) in &mut rigs {
        if rig.model != Model::Counselor || rig.bones.is_empty() || players.get(parent.parent()).map_or(true, |p| p.counselor.alive()) {
            continue;
        }
        let Some(mut r) = build(&character.info, rig, &transforms, &globals) else {
            warn!("no ragdoll: the counselor export has no physics asset (re-export with tools/f13-export counselor)");
            let mut sim = Ragdoll::default();
            sim.asleep = true;
            commands.entity(e).insert(Ragdolled { sim, bones: Vec::new(), rest: HashMap::new(), age: 0.0 });
            continue;
        };
        r.sim.settle(&collision.with(&dynamic), 20);
        info!("ragdoll: {} bodies, {} links, {} joints", r.sim.bodies.len(), r.sim.links.len(), r.sim.cones.len());
        debug!("ragdoll joints at the start: {}", r.angles());
        for (c, (s1, s2)) in r.sim.cones.iter().zip(r.sim.swings()) {
            debug!("ragdoll joint {}: swing {s1:.0} / {:.0}, {s2:.0} / {:.0}", r.bones[c.child].1, c.swing1.to_degrees(), c.swing2.to_degrees());
        }
        anim.stop_all();
        commands.entity(e).insert(r);
    }
}

fn side_name(name: &str) -> Option<(&str, bool)> {
    name.strip_suffix("_l").map(|s| (s, false)).or_else(|| name.strip_suffix("_r").map(|s| (s, true)))
}

/// An Unreal bone-space direction in the glTF bone space the rig uses.
fn ue_dir(v: [f32; 3]) -> Vec3 {
    let x = Xform { t: [v[0] * 100.0, v[1] * 100.0, v[2] * 100.0], ..Xform::IDENTITY };
    crate::to_transform(&x).translation.normalize_or(Vec3::X)
}

fn build(info: &CharacterInfo, rig: &Rig, transforms: &Query<&Transform>, globals: &Query<&GlobalTransform>) -> Option<Ragdolled> {
    if info.ragdoll.is_empty() {
        return None;
    }
    let index: HashMap<&str, usize> = info.bones.iter().enumerate().map(|(i, b)| (b.name.as_str(), i)).collect();
    let world = |bone: &str| rig.bones.get(bone).and_then(|&e| globals.get(e).ok()).map(|g| g.compute_transform());
    let dt = 1.0 / 64.0;
    let mut sim = Ragdoll::default();
    // One particle per body, at its bone.
    let mut particle: HashMap<&str, usize> = HashMap::new();
    let mut radius: HashMap<&str, f32> = HashMap::new();
    for b in &info.ragdoll {
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
                    local = shape.rotation * Vec3::Z * (b.length / 200.0 + radius[b.bone.as_str()]);
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
            subtree: Vec::new(),
        };
        body_of.insert(b.bone.as_str(), sim.bodies.len());
        sim.bodies.push(body);
        bones.push((bone, b.bone.clone(), None, xf.scale));
    }

    // What turns with each body: its aim (a tip) and every body below it, with their tips.
    let tips: Vec<Option<usize>> = sim.bodies.iter().map(|b| (b.aim != b.particle && !sim.bodies.iter().any(|o| o.particle == b.aim)).then_some(b.aim)).collect();
    for i in 0..sim.bodies.len() {
        let mut subtree = Vec::new();
        let mut stack = vec![info.ragdoll.iter().filter(|b| body_of.contains_key(b.bone.as_str())).nth(i).map(|b| b.bone.as_str()).unwrap_or_default()];
        let mut first = true;
        while let Some(bone) = stack.pop() {
            let Some(&bi) = body_of.get(bone) else { continue };
            if !first {
                subtree.push(sim.bodies[bi].particle);
            }
            first = false;
            subtree.extend(tips[bi]);
            stack.extend(children(bone));
        }
        sim.bodies[i].subtree = subtree;
    }

    // Joints: the child hangs off its parent, and swings within the joint's cone.
    for j in &info.joints {
        let (Some(&c), Some(&p)) = (body_of.get(j.bone1.as_str()), body_of.get(j.bone2.as_str())) else { continue };
        bones[c].2 = Some(p);
        sim.rigid(sim.bodies[p].particle, sim.bodies[c].particle);
        let (pri2, sec2) = (ue_dir(j.pri2), ue_dir(j.sec2));
        let z2 = pri2.cross(sec2).normalize_or(Vec3::Z);
        sim.cones.push(Cone {
            child: c,
            parent: p,
            x1: ue_dir(j.pri1),
            frame2: Mat3::from_cols(pri2, z2.cross(pri2), z2),
            swing1: j.swing1.to_radians(),
            swing2: j.swing2.to_radians(),
        });
    }
    // The hips and the shoulders hold their width.
    for (l, r) in sim.bodies.iter().filter_map(|b| b.sides).collect::<Vec<_>>() {
        sim.rigid(l, r);
    }

    let rest = info
        .bones
        .iter()
        .enumerate()
        .filter_map(|(i, b)| rig.bones.get(&b.name).and_then(|&e| transforms.get(e).ok()).map(|t| (i, *t)))
        .collect();
    Some(Ragdolled { sim, bones, rest, age: 0.0 })
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
            let over: Vec<String> = r
                .sim
                .cones
                .iter()
                .zip(r.sim.swings())
                .filter(|(c, (a, b))| a.abs() > c.swing1.to_degrees().max(10.0) + 3.0 || b.abs() > c.swing2.to_degrees().max(10.0) + 3.0)
                .map(|(c, (a, b))| format!("{} {a:.0}/{:.0} {b:.0}/{:.0}", r.bones[c.child].1, c.swing1.to_degrees(), c.swing2.to_degrees()))
                .collect();
            trace!("ragdoll t={:.1}: over the limits: {}", r.age, over.join(", "));
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
    rigs: Query<(&Rig, &Ragdolled)>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
    mut transforms: Query<&mut Transform>,
) {
    let Some(character) = character else { return };
    let info = &character.info;
    for (rig, r) in &rigs {
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
            let Some(&e) = rig.bones.get(&bone.name) else { continue };
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
