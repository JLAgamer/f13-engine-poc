//! The match's runtime actors ([`f13_data::world::MatchWorld`]): renders everything spawned at match start (doors,
//! furniture, cabinets, items, cars) and tags the gameplay ones (items, cabinets, vehicles, escape zones).

use crate::to_transform;
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;
use f13_core::spawn::SpawnKind;
use f13_data::classes::ClassDb;
use f13_data::level::{LevelLayout, Xform};
use f13_data::world::MatchWorld;
use std::collections::HashMap;

/// A pickup. `large` items are carried in the hands, small ones in the pockets.
#[derive(Component, Debug)]
pub struct Item {
    pub class: String,
    pub name: String,
    pub large: bool,
    /// Interaction range in cm (the item's interact component `DistanceLimit`).
    pub reach: f32,
    pub kind: ItemKind,
    /// The counselor's socket it's held by (`HandAttachSocket`, else SCItem's WeaponSocket).
    pub hand_socket: String,
    /// The cue it makes when put down (DroppedSoundCue).
    pub drop_cue: Option<String>,
    /// How many of its class she can carry (InventoryMax; 0: no limit).
    pub inventory_max: u32,
    /// bIsSpecial (SCSpecialItem's constructor sets it: the map): kept apart from her pockets.
    pub special: bool,
    /// Its idle pose in her hands (SCItem::GetIdlePose: the first IdleAsset on her skeleton; they all are) and a
    /// gun's LoadedIdle (SCGun's override while it has ammunition).
    pub idle_pose: Option<String>,
    pub loaded_idle: Option<String>,
}

/// SCImpactEffect's FX fields in order (class_layouts.json: DefaultFX 0x48, CounselorFX 0x70, JasonFX 0x98, then
/// WoodHeavyFX 0xc0 .. WoodDoorFX 0x408 every 0x28: surface types 1..22).
pub const IMPACT_FX: [&str; 25] = [
    "DefaultFX",
    "CounselorFX",
    "JasonFX",
    "WoodHeavyFX",
    "WoodMediumFX",
    "WoodLightFX",
    "WoodHollowFX",
    "SandFX",
    "GravelFX",
    "DirtDryFX",
    "DirtWetFX",
    "MudFX",
    "GrassFX",
    "LeafLitterFX",
    "WaterDeepFX",
    "WaterShallowFX",
    "MetalThickFX",
    "MetalMediumFX",
    "MetalLightFX",
    "MetalHollowFX",
    "RockFX",
    "LargeFoliageFX",
    "SmallFoliageFX",
    "ClothFX",
    "WoodDoorFX",
];

/// What an item does (by its native class).
#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    /// SCWeapon: swung at Jason.
    Weapon(WeaponStats),
    /// SCGun: the flare gun (a projectile) or the shotgun (pellets); `melee`: it has attack montages (NeutralAttack:
    /// the shotgun swings as a two-handed weapon in combat stance, loaded or not; the flare gun's are None).
    Gun { stats: WeaponStats, projectile: bool, pellets: u32, spread_deg: f32, velocity: f32, melee: bool, impacts: Vec<Option<String>>, impact_sounds: Vec<Option<String>>, impact_decals: Vec<Option<crate::decals::ImpactDecal>> },
    /// SCTrap: placed on the ground.
    Trap,
    FirstAid,
    /// SCNoiseMaker: the firecrackers, thrown `throw_delay` seconds into the throw (ThrowDelayTime).
    Firecracker { throw_delay: f32 },
    /// Breaks Jason's grab.
    PocketKnife,
    Map,
    WalkieTalkie,
    /// Car parts and anything else used elsewhere.
    Other,
}

/// A weapon's WeaponStats.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponStats {
    pub damage: f32,
    pub durability: f32,
    pub attacking_wear: f32,
    pub blocking_wear: f32,
    /// Percent.
    pub stun_chance: f32,
    pub stun: String,
    pub two_handed: bool,
    /// Its KillVolume (the "DamageVolume" capsule): actor-space transform (its axis is the local Y), radius and half
    /// height in metres.
    pub volume: Option<(Transform, f32, f32)>,
}

impl ItemKind {
    fn of(classes: &ClassDb, class: &str) -> Self {
        let p = |n: &str| classes.prop(class, n);
        let stats = || {
            let w = p("WeaponStats");
            let f = |n: &str, d: f32| w[n].as_f64().map_or(d, |v| v as f32);
            WeaponStats {
                damage: f("Damage", 10.0),
                durability: f("Durability", 30.0),
                attacking_wear: f("AttackingWear", 5.0),
                blocking_wear: f("BlockingWear", 2.0),
                stun_chance: f("StunChance", 0.0),
                stun: w["WeaponStunClass"].as_str().unwrap_or("BaseWeaponStun_C").to_string(),
                two_handed: p("bIsTwoHanded").as_bool().unwrap_or(false),
                volume: classes.get(class).and_then(|i| i.components.iter().find(|c| c.name == "DamageVolume")).map(|c| {
                    let r = |n: &str| c.props[n].as_f64().unwrap_or(0.0) as f32 / 100.0;
                    (c.local.as_ref().map(crate::to_transform).unwrap_or_default(), r("CapsuleRadius"), r("CapsuleHalfHeight"))
                }),
            }
        };
        let is = |n: &str| classes.is_child_of(class, n);
        if is("SCGun") {
            ItemKind::Gun {
                stats: stats(),
                projectile: !p("ProjectileClass").is_null(),
                pellets: p("NumPellets").as_u64().unwrap_or(1) as u32,
                spread_deg: p("MaxSpreadAngle").as_f64().unwrap_or(1.0) as f32,
                velocity: p("ProjectileVelocity").as_f64().unwrap_or(6000.0) as f32,
                melee: p("NeutralAttack").as_array().is_some_and(|a| a.iter().any(|x| x["AttackMontage"].is_string())),
                // Its PelletImpactTemplate's FX (an SCImpactEffect): DefaultFX, CounselorFX, JasonFX, then one per
                // surface type 1..22 (GetImpactFX FUN_140345dc0: 0xc0 + 0x28 (n - 1)).
                impacts: p("PelletImpactTemplate").as_str().map_or_else(Vec::new, |t| {
                    IMPACT_FX.iter().map(|f| classes.prop(t, f)["AssetPathName"].as_str().and_then(|a| a.rsplit('.').next()).map(String::from)).collect()
                }),
                // And its sounds (GetImpactSound FUN_140347cb0): DefaultSound, then surface types 1..22 (0x4d0 ..).
                impact_sounds: p("PelletImpactTemplate").as_str().map_or_else(Vec::new, |t| {
                    std::iter::once("DefaultSound".to_string())
                        .chain(IMPACT_FX[3..].iter().map(|f| f.replace("FX", "Sound")))
                        .map(|f| classes.prop(t, &f)["AssetPathName"].as_str().and_then(|a| a.rsplit('.').next()).map(String::from))
                        .collect()
                }),
                // And its decals (GetImpactDecal FUN_1403413b0): DefaultDecal 0x868, CounselorDecal, JasonDecal, then
                // surface types 1..22 (0x910 + 0x38 (n - 1)).
                impact_decals: p("PelletImpactTemplate").as_str().map_or_else(Vec::new, |t| {
                    IMPACT_FX.iter().map(|f| crate::decals::ImpactDecal::from_json(classes.prop(t, &f.replace("FX", "Decal")))).collect()
                }),
            }
        } else if is("SCWeapon") {
            ItemKind::Weapon(stats())
        } else if is("SCTrap") {
            ItemKind::Trap
        } else if is("SCFirstAid") {
            ItemKind::FirstAid
        } else if is("SCNoiseMaker") {
            ItemKind::Firecracker { throw_delay: p("ThrowDelayTime").as_f64().unwrap_or(0.8) as f32 }
        } else if is("SCPocketKnife") {
            ItemKind::PocketKnife
        } else if is("SCWalkieTalkie") {
            ItemKind::WalkieTalkie
        } else if class.starts_with("Map") {
            ItemKind::Map
        } else {
            ItemKind::Other
        }
    }
}

/// Throwing knives lying about for Jason (SCThrowingKnifePickup: StackSize knives; counselors can't take them).
#[derive(Component)]
pub struct KnifePickup {
    pub count: u32,
    /// Interaction range, cm.
    pub reach: f32,
}

/// The item sits in this cabinet's drawer (`.1`: SCDoubleCabinet's second is 1); the cabinet hands it over
/// (cabinets.rs).
#[derive(Component)]
pub struct InCabinet(pub Entity, pub usize);

pub use crate::cabinets::Cabinet;

/// A cabinet's moving part (its AnimatedMesh: a drawer or a door).
#[derive(Component)]
pub struct Drawer;

/// A place on a vehicle where a part is installed (gas tank, hood, ignition).
#[derive(Debug, Clone)]
pub struct RepairPoint {
    pub name: String,
    pub parts: Vec<String>,
    /// Local (Bevy) position on the vehicle, and range in cm.
    pub at: Vec3,
    pub reach: f32,
    pub done: bool,
    /// `HoldTimeLimit` when the Blueprint sets one (otherwise the repair component default).
    pub hold_time: Option<f32>,
    /// The interactable has pips (the skill-check minigame).
    pub has_pips: bool,
    /// Her take of its CounselorInteractAnim montage, without the section (`f_normal_stand_car_repair_gascan`).
    pub anim: Option<String>,
    /// Where she stands for it (its SCSpecialMoveComponent, Bevy local; she faces its forward).
    pub stand: Option<Transform>,
}

#[derive(Component, Debug)]
pub struct Vehicle {
    pub class: String,
    /// Parts installed from outside (gas tank, hood). The driver's seat is usable once these are done.
    pub repairs: Vec<RepairPoint>,
    /// Part the driver needs at the ignition (`StartVehicleComponent`: keys). Only the driver can use it.
    pub ignition: Vec<String>,
    /// Driver door (Bevy local) and its range in cm.
    pub driver_seat: Vec3,
    pub seat_reach: f32,
    /// Where she sits driving (Bevy local): the driver's seat's AttachSocket on the car's mesh.
    pub sitting: Transform,
    /// Where Jason stands to pull the driver out (its DriverGrab context kill, Bevy local).
    pub driver_grab: Option<Transform>,
    pub started: bool,
    /// The driver's throttle (-1..1) and handbrake (the Blueprint's OnThrottle and OnHandbrake: car_lights.rs).
    pub throttle: f32,
    pub handbrake: bool,
    /// Forward speed in cm/s and current steering, -1..1.
    pub speed: f32,
    pub steer: f32,
    /// Its handling (f13-core::vehicle: the movement component's engine, gearbox and wheels) and drivetrain state.
    pub setup: f13_core::vehicle::VehicleSetup,
    pub drive: f13_core::vehicle::VehicleState,
    /// Its PhysX setup (f13-core vehicle_px), and the car simulated from it once it's first driven.
    pub px: Option<f13_core::vehicle_px::CarSetup>,
    pub sim: Option<f13_core::vehicle_px::Car>,
}

impl Vehicle {
    pub fn repaired(&self) -> bool {
        self.repairs.iter().all(|r| r.done)
    }
}

/// Meshes of a vehicle (so its own ray casts can skip them).
#[derive(Component)]
pub struct VehiclePart(pub Entity);

#[derive(Component, Debug)]
pub struct EscapeZone {
    pub car: bool,
    /// Open (MULTICAST_ActivateVolume): the cars' from the start, the police's when they arrive (police.rs).
    pub active: bool,
    pub radius_m: f32,
    /// The volume's trigger root: on the road just before the zone, so (zone - trigger) is roughly the way out.
    pub trigger: Vec3,
    /// Its ExitPath spline's points (world), the way out of the map.
    pub exit: Vec<Vec3>,
    /// EscapeDelay, s: from entering to escaping.
    pub delay: f32,
    /// Its EscapeCameraSpline (SCCameraSplineComponent: its curves and its frame in the world) and CameraLerpTime (s).
    pub camera: Option<(f13_core::camera_spline::CameraSpline, Transform)>,
    pub camera_lerp: f32,
}

pub struct PropsPlugin {
    pub seed: u64,
    /// Dev aid: start the counselor at the driver's door of the 2-seat car.
    pub near_car: bool,
}

#[derive(Resource)]
struct Seed(u64);

#[derive(Resource)]
struct NearCar;

impl Plugin for PropsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Seed(self.seed)).add_systems(Startup, spawn_props).add_systems(Update, tag_vehicle_parts);
        if self.near_car {
            app.insert_resource(NearCar).add_systems(Update, move_near_car.run_if(resource_exists::<NearCar>));
        }
    }
}

fn move_near_car(
    mut commands: Commands,
    vehicles: Query<(&Vehicle, &GlobalTransform)>,
    mut player: Single<&mut Transform, (With<crate::player::Player>, With<crate::controls::LocalPlayer>)>,
    mut camera: Single<&mut crate::player::PlayerCamera>,
) {
    let Some((v, gxf)) = vehicles.iter().find(|(v, _)| v.class.contains("2Seat")).or_else(|| vehicles.iter().next()) else { return };
    let seat = v.driver_seat;
    let out = Vec3::new(seat.x, 0.0, seat.z).normalize_or_zero();
    player.translation = gxf.transform_point(seat + out * 1.5);
    player.translation.y = gxf.translation().y;
    // Look at the car.
    let to_car = gxf.translation() - player.translation;
    camera.yaw = (-to_car.x).atan2(-to_car.z);
    let p = player.translation;
    info!("--near-car: placed at ({:.1}, {:.1}, {:.1}) beside {}", p.x, p.y, p.z, v.class);
    commands.remove_resource::<NearCar>();
}

/// A boolean class default (C++ constructor defaults aren't in the export, hence `default`).
fn actor_bool(classes: &ClassDb, class: &str, name: &str, default: bool) -> bool {
    classes.prop(class, name).as_bool().unwrap_or(default)
}

/// An FTransform property ({Translation, Rotation, Scale3D}, each part optional).
fn transform_prop(v: &serde_json::Value) -> Option<Xform> {
    let o = v.as_object()?;
    let f = |v: &serde_json::Value, k: &str, d: f32| v[k].as_f64().map_or(d, |x| x as f32);
    let (t, r, s) = (&o.get("Translation"), &o.get("Rotation"), &o.get("Scale3D"));
    Some(Xform {
        t: t.map_or([0.0; 3], |t| [f(t, "X", 0.0), f(t, "Y", 0.0), f(t, "Z", 0.0)]),
        r: r.map_or([0.0, 0.0, 0.0, 1.0], |r| [f(r, "X", 0.0), f(r, "Y", 0.0), f(r, "Z", 0.0), f(r, "W", 1.0)]),
        s: s.map_or([1.0; 3], |s| [f(s, "X", 1.0), f(s, "Y", 1.0), f(s, "Z", 1.0)]),
    })
}

fn reach(c: &f13_data::level::ComponentInfo, default: f32) -> f32 {
    c.props["DistanceLimit"].as_f64().map_or(default, |d| d as f32)
}

fn local(c: &f13_data::level::ComponentInfo) -> Transform {
    to_transform(&c.local.unwrap_or(Xform::IDENTITY))
}

/// A level-placed actor this module spawns as a runtime actor (its parts too): hiding spots (outhouses, tents).
pub(crate) fn is_level_runtime_actor(classes: &ClassDb, class: &str) -> bool {
    classes.is_child_of(class, "SCHidingSpot")
}

/// Builds the match world and spawns its runtime actors.
fn spawn_props(
    mut commands: Commands,
    assets: Res<AssetServer>,
    level: Res<crate::Level>,
    seed: Res<Seed>,
    export: Res<crate::ExportRoot>,
    collision: Option<Res<crate::collision::WorldCollision>>,
) {
    let classes = match ClassDb::load(&export.0.join("classes.json")) {
        Ok(c) => c,
        Err(e) => {
            warn!("no classes.json in the export ({e}); re-run tools/f13-export to get items, cabinets and cars");
            return;
        }
    };
    let layouts: &[LevelLayout] = &level.layouts;
    let settings = f13_data::world::spawn_settings(&MatchWorld::world_settings(layouts));
    let world = MatchWorld::build(layouts, &classes, &settings, seed.0);

    let mut scenes: HashMap<String, Handle<WorldAsset>> = HashMap::new();
    let mut scene = |file: &str| {
        scenes.entry(file.to_string()).or_insert_with(|| assets.load(GltfAssetLabel::Scene(0).from_asset(file.to_string()))).clone()
    };
    let mut entities: HashMap<usize, Entity> = HashMap::new();
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    let mut item_classes: std::collections::BTreeMap<String, usize> = Default::default();

    let level_actors = world.actors.iter().enumerate().filter(|(_, a)| a.from_level && is_level_runtime_actor(&classes, &a.class));
    for (id, actor) in world.spawned_actors().chain(level_actors) {
        let Some(info) = classes.get(&actor.class) else { continue };
        let root = commands.spawn((to_transform(&actor.transform), Visibility::default(), Name::new(actor.class.clone()))).id();
        entities.insert(id, root);
        commands.entity(root).insert(crate::net_world::WorldId(id as u32));
        // The cues its class names (doors, windows, barricades, cars...): audio.rs plays them.
        let sounds: HashMap<String, String> = info
            .props
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| Some((k.clone(), crate::audio::cue_name(v)?)))
            // And its audio components' Sounds, by component name (a car's HornAudioComponent, BrakeAudio...).
            .chain(info.components.iter().filter(|c| c.kind == "AudioComponent").filter_map(|c| Some((c.name.clone(), crate::audio::cue_name(&c.props["Sound"])?))))
            .collect();
        if !sounds.is_empty() {
            commands.entity(root).insert(crate::audio::ClassSounds(sounds));
        }
        if let Some(cams) = crate::interaction_camera::InteractionCameras::new(&info.components) {
            commands.entity(root).insert(cams);
        }
        let is_door = classes.is_child_of(&actor.class, "SCDoor");
        let pivot = is_door.then(|| info.components.iter().find(|c| c.name == "Pivot")).flatten().map(|p| {
            let t = local(p);
            (commands.spawn((t, Visibility::default(), ChildOf(root))).id(), t)
        });
        let mut leaf: Option<(Entity, Transform, Option<crate::materials::MaterialOverrides>)> = None;
        let mut parts: HashMap<&str, Entity> = HashMap::new();
        // Hiding spot doors swing on their pivots (hiding.rs).
        let is_hiding_spot = classes.is_child_of(&actor.class, "SCHidingSpot");
        let mut spot_pivots: HashMap<&str, (Entity, Transform)> = HashMap::new();
        if is_hiding_spot {
            for c in info.components.iter().filter(|c| c.name.ends_with("DoorPivot")) {
                let t = local(c);
                spot_pivots.insert(c.name.as_str(), (commands.spawn((t, Visibility::default(), ChildOf(root))).id(), t));
            }
        }
        for c in &info.components {
            let Some(mesh) = &c.mesh else { continue };
            if c.hidden || ["Destroyed", "Broken", "Broke"].iter().any(|s| c.name.contains(s)) {
                continue;
            }
            if let Some(&(p, pt)) = c.parent.as_deref().and_then(|n| spot_pivots.get(n)) {
                let rel = Transform::from_matrix(pt.to_matrix().inverse() * local(c).to_matrix());
                let door = commands.spawn((WorldAssetRoot(scene(mesh)), rel, ChildOf(p))).id();
                if let Some(o) = crate::materials::MaterialOverrides::new(&c.material_map) {
                    commands.entity(door).insert(o);
                }
                continue;
            }
            // Door leaves hang on the pivot (actor-space transforms re-expressed relative to it).
            if let (Some((p, pt)), Some("Pivot")) = (pivot, c.parent.as_deref()) {
                let rel = Transform::from_matrix(pt.to_matrix().inverse() * local(c).to_matrix());
                let e = commands.spawn((WorldAssetRoot(scene(mesh)), rel, ChildOf(p))).id();
                let overrides = crate::materials::MaterialOverrides::new(&c.material_map);
                if let Some(o) = &overrides {
                    commands.entity(e).insert(o.clone());
                }
                leaf = Some((e, rel, overrides));
                continue;
            }
            let child = commands.spawn((WorldAssetRoot(scene(mesh)), local(c), ChildOf(root))).id();
            if let Some(o) = crate::materials::MaterialOverrides::new(&c.material_map) {
                commands.entity(child).insert(o);
            }
            if let Some(col) = collision.as_ref().filter(|_| c.blocks_pawn && c.kind == "StaticMeshComponent").and_then(|w| w.mesh_collider(mesh)) {
                commands.entity(child).insert(col);
                if c.blocks_camera {
                    commands.entity(child).insert(crate::collision::CameraBlocker);
                }
            }
            parts.insert(c.name.as_str(), child);
            if c.name.starts_with("AnimatedMesh") {
                commands.entity(child).insert(Drawer);
            }
        }

        // Blocking shapes (a door's DoorCollision box, a car's body box): on the pivot when they hang off it. Spacer
        // capsules (in doorways, windows and car seats) only block while an interaction pushes characters clear.
        let mut hulls: HashMap<&str, Entity> = HashMap::new();
        for c in info.components.iter().filter(|c| c.hull.is_some() && !c.kind.contains("SpacerCapsule")) {
            let tris = crate::collision::hull_from_unreal(c.hull.as_deref().unwrap_or_default());
            if tris.is_empty() {
                continue;
            }
            let collider = crate::collision::Collider(std::sync::Arc::new(tris));
            let on_pivot = pivot.filter(|_| {
                let mut at = c.parent.as_deref();
                let mut depth = 0;
                while let Some(name) = at {
                    if name == "Pivot" {
                        return true;
                    }
                    at = info.components.iter().find(|x| x.name == name).and_then(|x| x.parent.as_deref());
                    depth += 1;
                    if depth > 16 {
                        break;
                    }
                }
                false
            });
            if let Some(&(p, pt)) = c.parent.as_deref().and_then(|n| spot_pivots.get(n)) {
                let rel = Transform::from_matrix(pt.to_matrix().inverse() * local(c).to_matrix());
                let e = commands.spawn((rel, Visibility::default(), collider, ChildOf(p))).id();
                if c.hull_blocks_camera {
                    commands.entity(e).insert(crate::collision::CameraBlocker);
                }
                if !c.hull_blocks_world_dynamic {
                    commands.entity(e).insert(crate::collision::NoWorldDynamic);
                }
                if !c.hull_blocks_visibility {
                    commands.entity(e).insert(crate::collision::NoVisibility);
                }
                continue;
            }
            match on_pivot {
                Some((p, pt)) => {
                    // A door leaf: doorways stay open on the navmesh (Jason opens or breaks the door on his way).
                    let rel = Transform::from_matrix(pt.to_matrix().inverse() * local(c).to_matrix());
                    let e = commands.spawn((rel, Visibility::default(), collider, crate::navigation::NotOnNavmesh, ChildOf(p))).id();
                    if c.hull_blocks_camera {
                        commands.entity(e).insert(crate::collision::CameraBlocker);
                    }
                    if !c.hull_blocks_world_dynamic {
                        commands.entity(e).insert(crate::collision::NoWorldDynamic);
                    }
                    if !c.hull_blocks_visibility {
                        commands.entity(e).insert(crate::collision::NoVisibility);
                    }
                }
                None => {
                    let e = commands.spawn((local(c), Visibility::default(), collider, ChildOf(root))).id();
                    if c.hull_blocks_camera {
                        commands.entity(e).insert(crate::collision::CameraBlocker);
                    }
                    if !c.hull_blocks_world_dynamic {
                        commands.entity(e).insert(crate::collision::NoWorldDynamic);
                    }
                    if !c.hull_blocks_visibility {
                        commands.entity(e).insert(crate::collision::NoVisibility);
                    }
                    hulls.insert(c.name.as_str(), e);
                }
            }
        }
        // A wall Jason can break through: its section (mesh and Collision box) goes when it gives way.
        if classes.is_child_of(&actor.class, "SCDestructableWall") {
            // Defaults: SCDestructableWall's constructor.
            let f = |name: &str, default: f32| classes.prop(&actor.class, name).as_f64().map_or(default, |v| v as f32);
            let section = info.components.iter().find(|c| c.name == "DestructibleSection");
            let interact = info.components.iter().find(|c| c.name == "Interactable");
            let damaged = ["Inside", "Outside"].map(|side| ["Light", "Medium", "Heavy"].map(|k| classes.prop(&actor.class, &format!("{k}Destruction{side}MeshFile")).as_str().map(&mut scene)));
            commands.entity(root).insert(crate::doors::Wall {
                health: f("BaseHealth", 150.0),
                base_health: f("BaseHealth", 150.0),
                thresholds: [f("LightDestructionThreshold", 0.8), f("MediumDestructionThreshold", 0.6), f("HeavyDestructionThreshold", 0.4)],
                broken: false,
                section: parts.get("DestructibleSection").copied(),
                collision: hulls.get("Collision").copied(),
                burst_force: f("BurstExplosionForce", 5000.0),
                hit_force: f("HitExplosionForce", 1000.0),
                lifespan: f("WallLifespanAfterDestruction", 10.0),
                section_xf: section.map(local).unwrap_or_default(),
                overrides: section.and_then(|c| crate::materials::MaterialOverrides::new(&c.material_map)),
                damaged,
                interact: interact.map_or(Vec3::ZERO, |c| local(c).translation),
                // ILLInteractableComponent's constructor: DistanceLimit 200.
                reach: interact.map_or(200.0, |c| reach(c, 200.0)),
                hit_particle: classes.prop(&actor.class, "HitParticle").as_str().and_then(|s| s.split('\'').nth(1)).map(str::to_string),
                destroy_particle: classes.prop(&actor.class, "DestroyParticle").as_str().and_then(|s| s.split('\'').nth(1)).map(str::to_string),
            });
            *counts.entry("walls").or_default() += 1;
            let p = to_transform(&actor.transform);
            debug!("wall at {:.1} facing {:.1}", p.translation, p.rotation * Vec3::X);
        }

        let limits = crate::interact::InteractLimits::of(&classes, &actor.class);
        if !limits.0.is_empty() {
            commands.entity(root).insert(limits);
        }
        let interact = info.components.iter().find(|c| c.kind == "SCInteractComponent" && c.name.starts_with("Interact"));
        // Interactions can exclude a side (e.g. Jason's throwing knives ignore counselors).
        let counselor_can_use = interact.is_none_or(|c| {
            !c.props["ActorIgnoreList"].as_array().into_iter().flatten().any(|a| a["AssetPathName"].as_str().is_some_and(|p| p.ends_with(".SCCounselorCharacter")))
        });
        if let Some((pivot, pivot_closed)) = pivot {
            let door_interact = info.components.iter().find(|c| c.name == "InteractComponent");
            commands.entity(root).insert(crate::doors::Door {
                spots: info.components.iter().filter(|c| c.name.ends_with("_SpecialMoveHandler")).map(|c| (c.name.clone(), local(c))).collect(),
                break_force: classes.prop(&actor.class, "BreakExplosionForce").as_f64().map_or(5000.0, |v| v as f32),
                burst_force: classes.prop(&actor.class, "BurstExplosionForce").as_f64().map_or(20000.0, |v| v as f32),
                out_arrow: info.components.iter().find(|c| c.name == "OutArrow").map_or(Vec3::ZERO, |c| local(c).translation),
                lifespan: classes.prop(&actor.class, "LifespanAfterDestruction").as_f64().map_or(10.0, |v| v as f32),
                open: false,
                locked: false,
                lockable: actor_bool(&classes, &actor.class, "bIsLockable", true),
                pivot,
                pivot_closed,
                interact: door_interact.map_or(Vec3::Y, |c| local(c).translation),
                reach: door_interact.map_or(200.0, |c| reach(c, 200.0)),
                hold_time: door_interact.and_then(|c| c.props["HoldTimeLimit"].as_f64()).map_or(0.5, |v| v as f32),
                angle: 0.0,
                target: 0.0,
                health: classes.prop(&actor.class, "BaseDoorHealth").as_f64().map_or(100.0, |v| v as f32),
                broken: false,
                swing_seconds: crate::doors::SWING_SECONDS,
                driver: None,
                base_health: classes.prop(&actor.class, "BaseDoorHealth").as_f64().map_or(100.0, |v| v as f32),
                damage_thresholds: [("Light", 0.75), ("Medium", 0.5), ("Heavy", 0.25)]
                    .map(|(k, d)| classes.prop(&actor.class, &format!("{k}DestructionThreshold")).as_f64().map_or(d, |v| v as f32)),
                visuals: leaf.take().map(|(leaf, leaf_xf, overrides)| crate::doors::DoorVisuals {
                    leaf,
                    leaf_xf,
                    overrides,
                    damaged: ["Light", "Medium", "Heavy"].map(|k| classes.prop(&actor.class, &format!("{k}DestructionMeshFile")).as_str().map(&mut scene)),
                }),
            });
            *counts.entry("doors").or_default() += 1;
        }
        if is_hiding_spot {
            // The left pivot opens one way, the right one the other.
            let mut doors: Vec<(Entity, Transform, f32)> = spot_pivots.iter().map(|(n, &(e, t))| (e, t, if n.starts_with("Right") { 1.0 } else { -1.0 })).collect();
            // The outhouse's door is its skeletal mesh (turned on its origin, the hinge; out toward its front, +X).
            if classes.prop(&actor.class, "HidingSpotType").as_str() == Some("ESCHidingSpotType::Outhouse") {
                if let (Some(&e), Some(c)) = (parts.get("skeletalMesh"), info.components.iter().find(|c| c.name == "skeletalMesh")) {
                    doors.push((e, local(c), -1.0));
                }
            }
            if let Some(mut spot) = crate::hiding::HidingSpot::new(&classes, &actor.class, local, doors) {
                if spot.kind == crate::hiding::SpotKind::Tent {
                    spot.prop = parts.get("skeletalMesh").copied();
                }
                commands.entity(root).insert(spot);
                *counts.entry("hiding spots").or_default() += 1;
            }
        }
        if is_door {
            // What a barricade's overlap finds of the door (SCBarricade's BeginPlay links its door that way).
            let shapes: Vec<_> = info
                .components
                .iter()
                .filter_map(|c| {
                    let tris = crate::collision::hull_from_unreal(c.query_hull.as_deref()?);
                    (!tris.is_empty()).then(|| crate::collision::transform_tris(&tris, &local(c).compute_affine()))
                })
                .collect();
            commands.entity(root).insert(crate::collision::QueryShapes(shapes));
        }
        if classes.is_child_of(&actor.class, "SCBarricade") {
            // "Mesh" (`..._01_b`) is the plank, stood on end beside the door; the component named
            // `S_Interactive_Door_Barricade_01_b` (mesh `..._01_a`) is the wall brackets it drops into.
            let plank = info.components.iter().find(|c| c.name == "Mesh").and_then(|c| Some((*parts.get("Mesh")?, c.local.unwrap_or(Xform::IDENTITY))));
            commands.entity(root).insert(crate::windows::Barricade { enabled: false, door: None, plank });
            *counts.entry("barricades").or_default() += 1;
        }
        // A cabin's doors and windows: its SetSenseEffects (Cabin_Base) lights them for Sense (sense_view.rs).
        if classes.is_child_of(&actor.class, "BP_DoorBase_C") || classes.is_child_of(&actor.class, "Cabin_Window_BP_C") {
            let owner = actor.spawned_by.map(|by| world.spawners[by].owner).filter(|&o| classes.is_child_of(&world.actors[o].class, "Cabin_Base_C"));
            if let Some(o) = owner {
                commands.entity(root).insert(crate::sense_view::CabinPart(o));
            }
        }
        if classes.is_child_of(&actor.class, "SCWindow") {
            let wi = info.components.iter().find(|c| c.kind == "SCInteractComponent");
            let f = |name: &str, default: f32| classes.prop(&actor.class, name).as_f64().map_or(default, |v| v as f32);
            // The moving pane slides to OpenLocation, relative to its parent (the frame).
            let pane = info.components.iter().find(|c| c.name == "Moveable Pane").and_then(|c| {
                let e = *parts.get(c.name.as_str())?;
                let closed = local(c);
                let parent = c.parent.as_deref().and_then(|p| info.components.iter().find(|x| x.name == p)).map_or(Transform::IDENTITY, local);
                let open_ue = classes.prop(&actor.class, "OpenLocation");
                let d = |k: &str| open_ue[k].as_f64().unwrap_or(0.0) as f32 / 100.0;
                let offset = parent.rotation * Vec3::new(d("X"), d("Z"), d("Y"));
                Some((e, closed, Transform { translation: closed.translation + offset, ..closed }))
            });
            let glass = ["Moveable Pane", "StaticPane"].iter().filter_map(|n| parts.get(n).copied()).collect();
            commands.entity(root).insert(crate::windows::Window {
                open: false,
                destroyed: false,
                interact: wi.map_or(Vec3::Y * 1.6, |c| local(c).translation),
                reach: wi.map_or(150.0, |c| reach(c, 150.0)),
                dive_damage: f("DiveDamage", 20.0),
                vault_damage: f("VaultDamage", 15.0),
                take_vault_damage: classes.prop(&actor.class, "bTakeVaultDamage").as_bool().unwrap_or(false),
                pane,
                glass,
                vault: ["CounselorVaultInside", "CounselorVaultOutside"].map(|n| info.components.iter().find(|c| c.name == n).map(local)),
                dive: ["CounselorDiveInside", "CounselorDiveOutside"].map(|n| info.components.iter().find(|c| c.name == n).map(local)),
            });
            *counts.entry("windows").or_default() += 1;
        }
        // A placed kill from a cabin's spawner.
        if let Some(k) = crate::placed_kills::placed_kill(&classes, &actor.class, &actor.transform, &serde_json::Value::Null, |c| parts.get(c).copied()) {
            commands.entity(root).insert(k);
            *counts.entry("placed kills").or_default() += 1;
        }
        if classes.is_child_of(&actor.class, "SCItem") && counselor_can_use {
            let large = f13_core::spawn::ClassQuery::is_large(&classes, &actor.class);
            commands.entity(root).insert(Item {
                class: actor.class.clone(),
                name: classes.display_name(&actor.class),
                large,
                reach: interact.map_or(150.0, |c| reach(c, 150.0)),
                kind: ItemKind::of(&classes, &actor.class),
                hand_socket: classes.prop(&actor.class, "HandAttachSocket").as_str().filter(|s| *s != "None").unwrap_or("WeaponSocket").to_string(),
                // A soft reference: "/Game/Sounds/ItemDrops/Name.Name".
                drop_cue: classes.prop(&actor.class, "DroppedSoundCue")["AssetPathName"].as_str().and_then(|p| p.rsplit('.').next()).map(String::from),
                inventory_max: classes.prop(&actor.class, "InventoryMax").as_u64().unwrap_or(0) as u32,
                special: classes.is_child_of(&actor.class, "SCSpecialItem"),
                idle_pose: classes.prop(&actor.class, "IdleAsset").as_array().and_then(|a| a.iter().find_map(|v| v.as_str().map(asset_name))),
                loaded_idle: classes.prop(&actor.class, "LoadedIdle").as_str().map(asset_name),
            });
            *item_classes.entry(actor.class.clone()).or_default() += 1;
            *counts.entry(if actor.kind == Some(SpawnKind::RepairPart) { "repair parts" } else { "items" }).or_default() += 1;
            if actor.kind == Some(SpawnKind::RepairPart) {
                let p = to_transform(&actor.transform).translation;
                info!("{} at ({:.0}, {:.0}, {:.0})", actor.class, p.x, p.y, p.z);
            }
        } else if classes.is_child_of(&actor.class, "SCThrowingKnifePickup") {
            let count = classes.prop(&actor.class, "StackSize").as_f64().unwrap_or(1.0).max(1.0) as u32;
            commands.entity(root).insert(KnifePickup { count, reach: interact.map_or(150.0, |c| reach(c, 150.0)) });
            *counts.entry("knife pickups").or_default() += 1;
            let p = to_transform(&actor.transform).translation;
            debug!("{} at ({:.1}, {:.1}, {:.1})", actor.class, p.x, p.y, p.z);
        } else if classes.is_child_of(&actor.class, "SCCabinet") {
            // Its drawers: SCCabinet's components, and SCDoubleCabinet's second set (named ...2).
            let component = |name: String| info.components.iter().find(|c| c.name == name);
            let follows = classes.prop(&actor.class, "bItemShouldFollowAnimatedMesh").as_bool().unwrap_or(false);
            let slots = ["", "2"]
                .into_iter()
                .filter_map(|s| {
                    let interact = component(format!("Interactable{s}"))?;
                    let handler = component(format!("UseCabinet_SpecialMoveHandler{s}"))?;
                    let mesh = component(format!("AnimatedMesh{s}"));
                    let close = transform_prop(&classes.prop(&actor.class, &format!("CloseTransform{s}"))).map(|x| to_transform(&x)).or(mesh.map(local)).unwrap_or_default();
                    let follow = follows.then(|| Some((local(component(format!("FollowLocation{s}"))?), local(component(format!("ItemAttachLocation{s}"))?)))).flatten();
                    Some(crate::cabinets::Slot::new(
                        local(interact).translation,
                        reach(interact, 100.0),
                        local(handler),
                        handler.props["SpecialMove"].as_str() == Some("Counselor_OpenDrawerLow_C"),
                        mesh.and_then(|m| parts.get(m.name.as_str()).copied()),
                        close,
                        follow,
                    ))
                })
                .collect();
            commands.entity(root).insert(Cabinet { slots });
            *counts.entry("cabinets").or_default() += 1;
        } else if classes.is_child_of(&actor.class, "SCDriveableVehicle") {
            let repairs = info
                .components
                .iter()
                .filter(|c| classes.is_child_of(&c.kind, "SCRepairComponent") && !classes.is_child_of(&c.kind, "SCVehicleStarterComponent"))
                .map(|c| RepairPoint {
                    name: c.name.clone(),
                    parts: f13_data::level::str_list(&c.props["RequiredPartClasses"]),
                    at: local(c).translation,
                    reach: reach(c, 150.0),
                    done: false,
                    hold_time: c.props["HoldTimeLimit"].as_f64().map(|v| v as f32),
                    has_pips: c.props["Pips"].as_array().is_some_and(|p| !p.is_empty()),
                    stand: info.components.iter().find(|s| s.parent.as_deref() == Some(c.name.as_str()) && classes.is_child_of(&s.kind, "SCSpecialMoveComponent")).map(local),
                    // Its CounselorInteractAnim (unset on the cars' repairs: AM_normal_stand_car_repair_<part> by the part).
                    anim: c.props["CounselorInteractAnim"].as_str().and_then(|s| s.split('\'').nth(1)).and_then(|m| m.strip_prefix("AM_")).map(|m| format!("f_{m}")).or_else(|| {
                        let parts = f13_data::level::str_list(&c.props["RequiredPartClasses"]);
                        let has = |w: &str| parts.iter().any(|p| p.contains(w));
                        if has("GasCan") {
                            Some("f_normal_stand_car_repair_gascan".into())
                        } else if has("Battery") {
                            Some("f_normal_stand_car_repair_battery".into())
                        } else {
                            None
                        }
                    }),
                })
                .collect();
            let seat = info.components.iter().find(|c| c.name == "DriverSeat");
            let ignition = info
                .components
                .iter()
                .filter(|c| classes.is_child_of(&c.kind, "SCVehicleStarterComponent"))
                .flat_map(|c| f13_data::level::str_list(&c.props["RequiredPartClasses"]))
                .collect();
            commands.entity(root).insert(Vehicle {
                class: actor.class.clone(),
                repairs,
                ignition,
                driver_seat: seat.map_or(Vec3::ZERO, |c| local(c).translation),
                seat_reach: seat.map_or(100.0, |c| reach(c, 100.0)),
                driver_grab: info.components.iter().find(|c| c.name == "DriverGrab").map(local),
                sitting: seat
                    .and_then(|s| {
                        let mesh = info.components.iter().find(|c| Some(&c.name) == s.parent.as_ref())?;
                        Some(local(mesh) * to_transform(mesh.sockets.get(s.props["AttachSocket"].as_str()?)?))
                    })
                    .unwrap_or_else(|| Transform::from_translation(seat.map_or(Vec3::ZERO, |c| local(c).translation))),
                started: false,
                throttle: 0.0,
                handbrake: false,
                speed: 0.0,
                steer: 0.0,
                setup: f13_data::vehicle::setup(&classes, &actor.class),
                drive: default(),
                px: f13_data::vehicle::px_setup(&classes, &actor.class),
                sim: None,
            });
            // Its lights (the Chevettes' Headlights and TailLights).
            let spots = crate::car_lights::spot_lights(&info.components);
            if !spots.is_empty() {
                commands.entity(root).insert(crate::car_lights::CarLights::new(&classes, &actor.class, &format!("{root}"))).with_children(|c| {
                    for (s, cookie) in spots {
                        let mut e = c.spawn(s);
                        if let Some(k) = cookie {
                            e.insert(k);
                        }
                    }
                });
            }
            // Its particle systems (the tire dust, the hood smoke), following it; named <root entity>.<component> for the
            // Blueprint's SetActive calls (car_lights.rs).
            for c in info.components.iter().filter(|c| c.kind == "ParticleSystemComponent") {
                let Some(system) = c.props["Template"].as_str().and_then(|t| t.split('\'').nth(1)) else { continue };
                commands.write_message(crate::particles::SpawnParticles {
                    system: system.to_string(),
                    transform: local(c),
                    follow: Some(root),
                    actor: format!("{root}.{}", c.name),
                    active: c.props["bAutoActivate"].as_bool() != Some(false),
                });
            }
            let p = to_transform(&actor.transform).translation;
            info!("{} at ({:.0}, {:.0}, {:.0})", actor.class, p.x, p.y, p.z);
            *counts.entry("vehicles").or_default() += 1;
        }
    }
    // Items inside cabinets.
    for (id, _) in world.spawned_actors() {
        if let (Some((cab, slot)), Some(&item)) = (world.cabinet_slot_of(id, &classes), entities.get(&id)) {
            if let Some(&cabinet) = entities.get(&cab) {
                commands.entity(item).insert(InCabinet(cabinet, slot));
            }
        }
    }
    // What the minimap shows: the runtime actors themselves (the cars), and markers for the level's (cabins).
    let mut icons = 0;
    for (id, a) in world.actors.iter().enumerate() {
        let Some(icon) = crate::ui::MapIcon::of(&classes, &a.class) else { continue };
        if let Some(&e) = entities.get(&id) {
            commands.entity(e).insert(icon);
        } else if a.from_level {
            commands.spawn((to_transform(&a.transform), icon, Name::new(format!("{} map icon", a.class))));
        } else {
            continue;
        }
        icons += 1;
    }
    *counts.entry("map icons").or_default() += icons;
    // Escape zones (level actors).
    for layout in layouts {
        for a in layout.actors_of("SCEscapeVolume") {
            let Some(sphere) = a.components.iter().find(|c| c.kind == "SphereComponent") else { continue };
            let Some(at) = sphere.world else { continue };
            let radius = sphere.props["SphereRadius"].as_f64().unwrap_or(32.0) as f32;
            let car = a.props["EscapeType"].as_str() == Some("ESCEscapeType::Car");
            let trigger = a.components.iter().find(|c| c.name == "TriggerRoot").and_then(|c| c.world).or(a.transform).unwrap_or(at);
            let exit = a
                .components
                .iter()
                .find(|c| c.name == "ExitPath")
                .and_then(|c| Some((c.world?, c.props["SplineCurves"]["Position"]["Points"].as_array()?)))
                .map(|(w, points)| {
                    points
                        .iter()
                        .filter_map(|p| {
                            let v = &p["OutVal"];
                            let t = [v["X"].as_f64()? as f32, v["Y"].as_f64()? as f32, v["Z"].as_f64()? as f32];
                            Some(to_transform(&Xform { t, r: [0.0, 0.0, 0.0, 1.0], s: [1.0; 3] }.then(&w)).translation)
                        })
                        .collect()
                })
                .unwrap_or_default();
            commands.spawn((
                to_transform(&Xform { r: [0.0, 0.0, 0.0, 1.0], s: [1.0; 3], ..at }),
                EscapeZone {
                    car,
                    active: car,
                    radius_m: radius / 100.0,
                    trigger: to_transform(&trigger).translation,
                    exit,
                    delay: a.props["EscapeDelay"].as_f64().map_or(0.0, |d| d as f32),
                    camera: a
                        .components
                        .iter()
                        .find(|c| c.name == "EscapeCameraSpline")
                        // At the actor's root (the export gives the native component no transform of its own).
                        .and_then(|c| Some((f13_data::kill_camera::parse(&c.props["SplineCurves"]), to_transform(&c.world.or(a.transform)?)))),
                    camera_lerp: a.props["CameraLerpTime"].as_f64().map_or(0.0, |d| d as f32),
                },
                Name::new(a.name.clone()),
            ));
        }
    }
    info!("match (seed {}): {} runtime actors, {:?}", seed.0, entities.len(), counts);
    info!("items: {:?}", item_classes);
}

/// Tags every mesh under a vehicle with [`VehiclePart`] once its scenes have spawned.
fn tag_vehicle_parts(
    mut commands: Commands,
    vehicles: Query<Entity, With<Vehicle>>,
    children: Query<&Children>,
    tagged: Query<(), With<VehiclePart>>,
) {
    for v in &vehicles {
        for e in children.iter_descendants(v) {
            if !tagged.contains(e) {
                commands.entity(e).insert(VehiclePart(v));
            }
        }
    }
}

/// An object reference's asset name ("AnimSequence'BatGrip'" -> "BatGrip").
fn asset_name(v: &str) -> String {
    v.split('\'').nth(1).unwrap_or(v).rsplit('.').next().unwrap_or(v).to_string()
}
