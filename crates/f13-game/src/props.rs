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
}

/// The item sits in this cabinet and can only be taken once the cabinet is open.
#[derive(Component)]
pub struct InCabinet(pub Entity);

#[derive(Component)]
pub struct Cabinet {
    pub open: bool,
    /// Interact point in the cabinet's local (Bevy) space, and its range in cm.
    pub interact: Vec3,
    pub reach: f32,
    pub drawer: Option<Entity>,
}

/// The cabinet's moving part (drawer or door) with its closed transform.
#[derive(Component)]
pub struct Drawer {
    pub closed: Transform,
}

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
    pub started: bool,
    /// Forward speed in cm/s and current steering, -1..1.
    pub speed: f32,
    pub steer: f32,
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
    pub radius_m: f32,
    /// The volume's trigger root: on the road just before the zone, so (zone - trigger) is roughly the way out.
    pub trigger: Vec3,
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
    mut player: Single<&mut Transform, With<crate::player::Player>>,
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

fn reach(c: &f13_data::level::ComponentInfo, default: f32) -> f32 {
    c.props["DistanceLimit"].as_f64().map_or(default, |d| d as f32)
}

fn local(c: &f13_data::level::ComponentInfo) -> Transform {
    to_transform(&c.local.unwrap_or(Xform::IDENTITY))
}

/// Builds the match world and spawns its runtime actors.
fn spawn_props(mut commands: Commands, assets: Res<AssetServer>, level: Res<crate::Level>, seed: Res<Seed>, export: Res<crate::ExportRoot>) {
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

    for (id, actor) in world.spawned_actors() {
        let Some(info) = classes.get(&actor.class) else { continue };
        if classes.is_child_of(&actor.class, "SCBarricade") {
            continue; // its open/closed/destroyed meshes overlap until barricades are implemented
        }
        let root = commands.spawn((to_transform(&actor.transform), Visibility::default(), Name::new(actor.class.clone()))).id();
        entities.insert(id, root);
        let mut drawer = None;
        for c in &info.components {
            let Some(mesh) = &c.mesh else { continue };
            if c.hidden || ["Destroyed", "Broken", "Broke"].iter().any(|s| c.name.contains(s)) {
                continue;
            }
            let child = commands.spawn((WorldAssetRoot(scene(mesh)), local(c), ChildOf(root))).id();
            if c.name == "AnimatedMesh" {
                commands.entity(child).insert(Drawer { closed: local(c) });
                drawer = Some(child);
            }
        }

        let interact = info.components.iter().find(|c| c.kind == "SCInteractComponent" && c.name.starts_with("Interact"));
        // Interactions can exclude a side (e.g. Jason's throwing knives ignore counselors).
        let counselor_can_use = interact.is_none_or(|c| {
            !c.props["ActorIgnoreList"].as_array().into_iter().flatten().any(|a| a["AssetPathName"].as_str().is_some_and(|p| p.ends_with(".SCCounselorCharacter")))
        });
        if classes.is_child_of(&actor.class, "SCItem") && counselor_can_use {
            let large = f13_core::spawn::ClassQuery::is_large(&classes, &actor.class);
            commands.entity(root).insert(Item {
                class: actor.class.clone(),
                name: classes.display_name(&actor.class),
                large,
                reach: interact.map_or(150.0, |c| reach(c, 150.0)),
            });
            *counts.entry(if actor.kind == Some(SpawnKind::RepairPart) { "repair parts" } else { "items" }).or_default() += 1;
            if actor.kind == Some(SpawnKind::RepairPart) {
                let p = to_transform(&actor.transform).translation;
                info!("{} at ({:.0}, {:.0}, {:.0})", actor.class, p.x, p.y, p.z);
            }
        } else if classes.is_child_of(&actor.class, "SCCabinet") {
            commands.entity(root).insert(Cabinet {
                open: false,
                interact: interact.map_or(Vec3::ZERO, |c| local(c).translation),
                reach: interact.map_or(100.0, |c| reach(c, 100.0)),
                drawer,
            });
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
                started: false,
                speed: 0.0,
                steer: 0.0,
            });
            let p = to_transform(&actor.transform).translation;
            info!("{} at ({:.0}, {:.0}, {:.0})", actor.class, p.x, p.y, p.z);
            *counts.entry("vehicles").or_default() += 1;
        }
    }
    // Items inside cabinets.
    for (id, _) in world.spawned_actors() {
        if let (Some(cab), Some(&item)) = (world.cabinet_of(id, &classes), entities.get(&id)) {
            if let Some(&cabinet) = entities.get(&cab) {
                commands.entity(item).insert(InCabinet(cabinet));
            }
        }
    }
    // Escape zones (level actors).
    for layout in layouts {
        for a in layout.actors_of("SCEscapeVolume") {
            let Some(sphere) = a.components.iter().find(|c| c.kind == "SphereComponent") else { continue };
            let Some(at) = sphere.world else { continue };
            let radius = sphere.props["SphereRadius"].as_f64().unwrap_or(32.0) as f32;
            let car = a.props["EscapeType"].as_str() == Some("ESCEscapeType::Car");
            let trigger = a.components.iter().find(|c| c.name == "TriggerRoot").and_then(|c| c.world).or(a.transform).unwrap_or(at);
            commands.spawn((
                to_transform(&Xform { r: [0.0, 0.0, 0.0, 1.0], s: [1.0; 3], ..at }),
                EscapeZone { car, radius_m: radius / 100.0, trigger: to_transform(&trigger).translation },
                Name::new(a.name.clone()),
            ));
        }
    }
    info!("match (seed {}): {} runtime actors, {:?}", seed.0, entities.len(), counts);
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
