//! Placed (environment) kills (SCContextKillActor: Wall_FaceSmash_PlacedKill, Table_Placedkill, ...): Jason holding
//! a counselor uses one he's at: he takes its ContextKillComponent's place and facing (InteractorPositionRequirement
//! LocationAndRotation), the two play its special moves' paired montages (InteractorSpecialClass Jason_*,
//! VictimSpecialClass Counselor_*: bDieOnFinish), and its own camera spline (under the ContextKillComponent) films
//! it. Most can be used once (bOnlyUseableOnce). Their KillInfo scores Jason's Resourceful.
//!
//! Reach (the Interactable's range test, FUN_14022e940, vtable slot 0x518): his actor's location (his capsule's centre)
//! within DistanceLimit (ILLInteractableComponent's 200 cm unless set) of the Interactable, in 3D; with bUseYawLimit,
//! the direction from it to him within MaxYawOffset (15 unless set) of its facing, and (his manager's
//! bUseCameraRotation, the constructor's true) his view within MaxYawOffset of facing back at it; with bUsePitchLimit
//! the same in pitch against MaxPitchOffset (90 unless set; here his view's pitch is taken as level). Its
//! bCheckForOcclusion isn't set on any placed kill.
//!
//! The bird bath, the water pump and the chair animate as the kill starts (their Blueprints' BeginPlay sets the
//! KillMesh and KillMeshAnimation, SetAnimatedKillMesh 0x14035a8f0, which pauses it; KillStarted 0x14034d180 plays it
//! once and pauses it again at its end); the bird bath's Blueprint also turns its mesh to (0, 0, -90) on KillStarted.
//! Placed kills come from the level and from the cabins' spawners (runtime actors, props.rs).
//!
//! bShouldOrientTowardKiller (the bird bath, the tree and light post arm ripoffs, the campfire stick, the stump slam):
//! SCContextKillActor's tick (FUN_140361770) turns the actor, while the local killer holds a counselor outside a
//! context kill within its Interactable's HighlightDistance (flat), to face along the line from him to it (yaw only),
//! so its kill spot, camera and Interactable come round to him. Its IgnoredRotationComponents (each Blueprint's
//! visible mesh: BirdBathSM, the trees, CampFire, Cylinder) get the actor's negated rotation as their relative one.
//! Not ported: those meshes' own turn (to world yaw 0) as the actor turns.

use bevy::prelude::*;
use f13_data::classes::ClassDb;
use f13_data::level::Xform;

pub struct PlacedKillsPlugin;

impl Plugin for PlacedKillsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_placed_kills).add_systems(FixedUpdate, orient_toward_killer.before(crate::jason::update_jason));
    }
}

/// Each kind: its actor class, Jason's take, hers, and its KillInfo's KillName.
pub const KINDS: [(&str, &str, &str, &str); 15] = [
    ("Birdbath_PlacedKill_C", "cinematic_hand_birdbath", "cinematic_hand_birdbath", "Bird Bath"),
    ("HeadSmash_PlacedKill_C", "cinematic_f_gravestone_headsmash_01", "cinematic_gravestone_headsmash", "Head Smash"),
    ("IronFence_PlacedKill_C", "cinematic_hand_ironfence", "cinematic_hand_ironfence", "Iron Fence"),
    ("TreeArmRipoff_PlacedKill_C", "cinematic_hand_tree_arm_ripoff", "cinematic_hand_tree_arm_ripoff", "Tree Arm Ripoff"),
    ("LightPostArmRipoff_PlacedKill_C", "cinematic_hand_tree_arm_ripoff", "cinematic_hand_tree_arm_ripoff", "Tree Arm Ripoff"),
    ("Vertical_BackBreak_PlacedKil_C", "cinematic_surface_vertical_backbreak", "cinematic_surface_vertical_backbreak", "Vertical Back Break"),
    ("Wall_FaceSmash_PlacedKill_C", "cinematic_m_wall_facesmash_01", "cinematic_wall_facesmash", "Wall Face Smash"),
    ("WaterPump_PlacedKill_C", "cinematic_hand_waterpump", "cinematic_hand_waterpump", "Water Pump"),
    ("AdirondackChair_PlacedKill_C", "cinematic_f_adirondackchair_kill_01", "cinematic_adirondackchair_kill", "Chair Kill"),
    ("Stick_Campfire_PlacedKill_C", "cinematic_stick_campfire", "cinematic_stick_campfire", "Campfire"),
    ("TreeStumpSlam_PlacedKill_C", "cinematic_hand_treestump_faceslam", "cinematic_hand_treestump_faceslam", "Tree Stump"),
    ("Fireplace_PlacedKill_C", "cinematic_fireplace", "cinematic_fireplace", "Fireplace"),
    ("Table_Placedkill_C", "cinematic_hand_table", "cinematic_hand_table", "Table Smash"),
    ("TreeBranch_PlacedKill_C", "cinematic_treebranch_chesthole_01", "cinematic_treebranch_chesthole_01", "Tree Branch"),
    ("WallHeadPunch_PlacedKill_C", "cinematic_wall_headpunch_01", "cinematic_wall_headpunch_01", "Wall Head Punch"),
];

/// ILLInteractableComponent's constructor: DistanceLimit (cm), MaxYawOffset and MaxPitchOffset (degrees).
const DEFAULT_REACH_CM: f32 = 200.0;
const DEFAULT_MAX_YAW: f64 = 15.0;
const DEFAULT_MAX_PITCH: f64 = 90.0;

/// A placed kill's actor: where the level put it, where it is now (turned toward Jason: bShouldOrientTowardKiller),
/// its components' places in it (Bevy: the ContextKillComponent, the camera spline, the Interactable), and how near
/// he must be for it to turn (HighlightDistance, m).
#[derive(Debug, Clone)]
pub struct KillActor {
    pub root: Transform,
    pub now: Transform,
    kill: Transform,
    camera: Transform,
    interact: Transform,
    pub orient_m: Option<f32>,
}

/// A placed kill in the level.
#[derive(Component, Debug)]
pub struct PlacedKill {
    pub actor: KillActor,
    /// Index into [`KINDS`].
    pub kind: usize,
    /// Its kill camera's key (kill_cameras.json: the actor's package name).
    pub camera_key: String,
    /// Where Jason stands for it (his transform: the ContextKillComponent's, turned to face as characters do).
    pub spot: Transform,
    /// Its camera spline's frame (world).
    pub camera: Transform,
    /// Its Interactable: where, facing which way, how far it reaches (m), and its yaw and pitch limits (degrees).
    pub interact: Vec3,
    pub forward: Vec3,
    pub reach_m: f32,
    pub yaw_limit: Option<f32>,
    pub pitch_limit: Option<f32>,
    pub only_once: bool,
    pub used: bool,
    /// Its KillMesh, its KillMeshAnimation, and the transform its Blueprint gives the mesh as the kill starts.
    pub prop: Option<(Entity, &'static str, Option<Transform>)>,
}

/// The placed kills whose prop plays its own animation as the kill starts: the class, its KillMesh component, its
/// KillMeshAnimation, and the relative rotation (pitch, yaw, roll) its Blueprint sets on KillStarted.
pub const KILL_PROPS: [(&str, &str, &str, Option<[f32; 3]>); 3] = [
    ("Birdbath_PlacedKill_C", "BirdBathSM", "cinematic_hand_birdbath", Some([0.0, 0.0, -90.0])),
    ("WaterPump_PlacedKill_C", "WaterPumpSM", "A_Interactive_WaterPump_01_Kill", None),
    ("AdirondackChair_PlacedKill_C", "AdirondackChair", "cinematic_f_adirondackchair_kill_01", None),
];

/// A placed kill of `class` at `t` (its actor's own properties `actor_props`): None for other classes. `prop` is the
/// entity of its KillMesh component, when it has one.
pub fn placed_kill(classes: &ClassDb, class: &str, t: &Xform, actor_props: &serde_json::Value, prop: impl Fn(&str) -> Option<Entity>) -> Option<PlacedKill> {
    let kind = KINDS.iter().position(|k| k.0 == class)?;
    let info = classes.get(class)?;
    let comp = |name: &str| info.components.iter().find(|c| c.name == name);
    let local = |c: &f13_data::level::ComponentInfo| crate::to_transform(&c.local.unwrap_or(Xform::IDENTITY));
    let kill = info.components.iter().find(|c| c.kind == "SCContextKillComponent")?;
    let camera = info.components.iter().find(|c| c.kind == "SCCameraSplineComponent")?;
    let interact = comp("Interactable")?;
    let root = crate::to_transform(t);
    let orient = info.props["bShouldOrientTowardKiller"].as_bool().unwrap_or(false);
    let actor = KillActor {
        root,
        now: root,
        kill: local(kill),
        camera: local(camera),
        interact: local(interact),
        orient_m: orient.then(|| interact.props["HighlightDistance"].as_f64().unwrap_or(0.0) as f32 / 100.0),
    };
    let prop = KILL_PROPS.iter().find(|p| p.0 == class).and_then(|&(_, component, anim, rotation)| {
        let at_start = rotation.zip(comp(component)).map(|(r, c)| {
            let x = c.local.unwrap_or(Xform::IDENTITY);
            crate::to_transform(&Xform { r: f13_core::camera_spline::quaternion(Vec3::from_array(r)).to_array(), ..x })
        });
        Some((prop(component)?, anim, at_start))
    });
    let mut k = PlacedKill {
        actor,
        kind,
        camera_key: class.trim_end_matches("_C").to_string(),
        spot: Transform::IDENTITY,
        camera: Transform::IDENTITY,
        interact: Vec3::ZERO,
        forward: Vec3::X,
        reach_m: interact.props["DistanceLimit"].as_f64().map_or(DEFAULT_REACH_CM, |d| d as f32) / 100.0,
        yaw_limit: interact.props["bUseYawLimit"].as_bool().unwrap_or(false).then(|| interact.props["MaxYawOffset"].as_f64().unwrap_or(DEFAULT_MAX_YAW) as f32),
        pitch_limit: interact.props["bUsePitchLimit"].as_bool().unwrap_or(false).then(|| interact.props["MaxPitchOffset"].as_f64().unwrap_or(DEFAULT_MAX_PITCH) as f32),
        only_once: actor_props["bOnlyUseableOnce"].as_bool().or(info.props["bOnlyUseableOnce"].as_bool()).unwrap_or(false),
        used: false,
        prop,
    };
    k.place(root);
    Some(k)
}

impl PlacedKill {
    /// Puts the actor at `now`: its kill spot (turned to face as characters do), camera and Interactable with it.
    pub fn place(&mut self, now: Transform) {
        self.actor.now = now;
        let spot = now.mul_transform(self.actor.kill);
        self.spot = Transform { rotation: spot.rotation * Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), ..spot };
        self.camera = now.mul_transform(self.actor.camera);
        let iw = now.mul_transform(self.actor.interact);
        self.interact = iw.translation;
        self.forward = (iw.rotation * Vec3::X).normalize_or(Vec3::X);
    }

    /// The kill mesh's place (under the actor's entity, which stays where the level put it) as KillStarted sets its
    /// relative transform: relative to the actor as it is now.
    pub fn prop_start(&self, relative: Transform) -> Transform {
        Transform::from_matrix(self.actor.root.to_matrix().inverse() * self.actor.now.mul_transform(relative).to_matrix())
    }

    /// Jason, his capsule's centre at `center` and looking along `view` (ground plane), can use it.
    pub fn in_reach(&self, center: Vec3, view: Vec2) -> bool {
        if self.used && self.only_once {
            return false;
        }
        let d = center - self.interact;
        if d.length() > self.reach_m {
            return false;
        }
        // Yaw and pitch (degrees) of a direction, as Unreal's rotator of it.
        let yaw = |v: Vec3| v.z.atan2(v.x).to_degrees();
        let pitch = |v: Vec3| v.y.atan2(Vec2::new(v.x, v.z).length()).to_degrees();
        let off = |a: f32| ((a + 180.0).rem_euclid(360.0) - 180.0).abs();
        let back = -self.forward;
        let view = Vec3::new(view.x, 0.0, view.y);
        let within = |v: Vec3, f: Vec3| {
            self.yaw_limit.is_none_or(|l| off(yaw(v) - yaw(f)) <= l) && self.pitch_limit.is_none_or(|l| off(pitch(v) - pitch(f)) <= l)
        };
        within(d, self.forward) && (view == Vec3::ZERO || within(view, back))
    }

    pub fn takes(&self) -> (&'static str, &'static str, &'static str) {
        let (_, his, hers, name) = KINDS[self.kind];
        (his, hers, name)
    }
}

/// bShouldOrientTowardKiller (FUN_140361770): Jason holding a counselor (not in a kill) within HighlightDistance (flat)
/// of the actor turns it to face along the line from him to it.
fn orient_toward_killer(jasons: Query<(&crate::jason::Jason, &Transform)>, mut kills: Query<&mut PlacedKill>) {
    let Some((_, jxf)) = jasons.iter().find(|(j, _)| j.holding()) else { return };
    for mut k in &mut kills {
        let Some(reach) = k.actor.orient_m else { continue };
        let d = k.actor.root.translation - jxf.translation;
        let flat = Vec2::new(d.x, d.z);
        if flat.length() > reach || flat == Vec2::ZERO {
            continue;
        }
        // MakeRotFromX of the flat line: the actor's Unreal +X (Bevy +X) along it.
        let rotation = Quat::from_rotation_y((-flat.y).atan2(flat.x));
        if k.actor.now.rotation.angle_between(rotation) > 1e-4 {
            let now = Transform { rotation, ..k.actor.root };
            k.place(now);
        }
    }
}

fn spawn_placed_kills(mut commands: Commands, level: Res<crate::Level>, export: Res<crate::ExportRoot>, assets: Res<AssetServer>) {
    let Ok(classes) = ClassDb::load(&export.0.join("classes.json")) else { return };
    let (mut n, mut props) = (0, 0);
    for a in level.layouts.iter().flat_map(|l| &l.actors) {
        if !KINDS.iter().any(|k| k.0 == a.class) {
            continue;
        }
        let (Some(t), Some(info)) = (a.transform, classes.get(&a.class)) else { continue };
        let root = commands.spawn((crate::to_transform(&t), Visibility::default(), Name::new(a.name.clone()))).id();
        // Its skeletal meshes (the level's placements hold only its static ones).
        let mut parts = std::collections::HashMap::new();
        for c in info.components.iter().filter(|c| c.kind.contains("SkeletalMeshComponent") && !c.hidden) {
            let Some(mesh) = &c.mesh else { continue };
            let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(mesh.clone()));
            let local = crate::to_transform(&c.local.unwrap_or(Xform::IDENTITY));
            parts.insert(c.name.clone(), commands.spawn((bevy::world_serialization::WorldAssetRoot(scene), local, ChildOf(root))).id());
            props += 1;
        }
        match placed_kill(&classes, &a.class, &t, &a.props, |c| parts.get(c).copied()) {
            Some(k) => {
                commands.entity(root).insert(k);
                n += 1;
            }
            None => {
                commands.entity(root).despawn();
            }
        }
    }
    info!("placed kills: {n} ({props} skeletal props)");
}
