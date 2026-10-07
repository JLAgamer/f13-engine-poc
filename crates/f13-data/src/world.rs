//! Builds the actors of one match: the level's own actors, everything their "spawn on initialize" spawners create
//! (doors, cabinets, part spawners...), then the match-start spawning of vehicles, repair parts and items
//! ([`f13_core::spawn`]). Pure data, in Unreal space; the game turns it into entities.

use crate::classes::ClassDb;
use crate::level::{LevelLayout, Xform};
use f13_core::spawn::{spawn_match, Cabinet, CountedClass, Rng, SpawnKind, SpawnSettings, SpawnWorld, Spawner};

#[derive(Debug, Clone)]
pub struct MatchActor {
    pub class: String,
    /// World transform of the actor's root.
    pub transform: Xform,
    /// Placed in the level (its meshes are part of the level placements), as opposed to spawned at runtime.
    pub from_level: bool,
    /// The spawner that created it, if any.
    pub spawned_by: Option<usize>,
    pub kind: Option<SpawnKind>,
}

#[derive(Debug, Clone)]
pub struct MatchSpawner {
    /// Owning actor (index into [`MatchWorld::actors`]).
    pub owner: usize,
    pub component: String,
    pub world: Xform,
    pub actor_list: Vec<String>,
    pub spawn_on_initialize: bool,
    /// Actor it spawned.
    pub spawned: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct MatchWorld {
    pub actors: Vec<MatchActor>,
    pub spawners: Vec<MatchSpawner>,
}

/// UE `FRotator::Quaternion` (degrees; pitch about Y, yaw about Z, roll about X).
pub fn rotator_to_quat(pitch: f32, yaw: f32, roll: f32) -> [f32; 4] {
    let h = std::f32::consts::PI / 360.0;
    let (sp, cp) = (pitch * h).sin_cos();
    let (sy, cy) = (yaw * h).sin_cos();
    let (sr, cr) = (roll * h).sin_cos();
    [
        cr * sp * sy - sr * cp * cy,
        -cr * sp * cy - sr * cp * sy,
        cr * cp * sy - sr * sp * cy,
        cr * cp * cy + sr * sp * sy,
    ]
}

/// Reads the spawn lists and distances from the map's `SCWorldSettings` properties.
pub fn spawn_settings(world_settings: &serde_json::Value) -> SpawnSettings {
    let f = |k: &str| world_settings[k].as_f64().unwrap_or(0.0) as f32;
    let list = |v: &serde_json::Value| crate::level::str_list(v);
    let counted = |key: &str, class_key: &str| -> Vec<CountedClass> {
        world_settings[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| {
                Some(CountedClass {
                    class: e[class_key].as_str()?.to_string(),
                    min: e["MinCount"].as_i64().unwrap_or(0) as i32,
                    max: e["MaxCount"].as_i64().unwrap_or(0) as i32,
                    min_spawn_distance: e["MinSpawnDistance"].as_f64().unwrap_or(0.0) as f32,
                })
            })
            .collect()
    };
    SpawnSettings {
        vehicles: world_settings["VehiclesToSpawn"].as_array().into_iter().flatten().map(|e| list(&e["VehicleClasses"])).collect(),
        large_items: counted("LargeItemSpawnData", "ActorClass"),
        small_items: counted("SmallItemSpawnData", "ItemClass"),
        min_vehicle_spawn_distance: f("MinVehicleSpawnDistance"),
        min_vehicle_part_spawn_distance: f("MinVehiclePartSpawnDistance"),
        max_car_part_distance: f("MaxCarPartDistance"),
        max_fuse_distance: f("MaxFuseDistance"),
        min_fuse_spawn_distance: f("MinFuseSpawnDistance"),
    }
}

impl MatchWorld {
    /// The map's world settings: the `SCWorldSettings` actor that has spawn data (the persistent level's).
    pub fn world_settings(layouts: &[LevelLayout]) -> serde_json::Value {
        layouts
            .iter()
            .flat_map(|l| l.actors_of("SCWorldSettings"))
            .find(|a| !a.props["VehiclesToSpawn"].is_null())
            .map(|a| a.props.clone())
            .unwrap_or(serde_json::Value::Null)
    }

    /// Builds the match. `settings` usually comes from [`spawn_settings`]; the seed makes it reproducible.
    pub fn build(layouts: &[LevelLayout], classes: &ClassDb, settings: &SpawnSettings, seed: u64) -> MatchWorld {
        /// World transform of component `name` of a placed `class` actor whose root is at `actor`.
        fn class_component_world(classes: &ClassDb, class: &str, name: &str, actor: &Xform) -> Option<Xform> {
            let c = classes.get(class)?.components.iter().find(|c| c.name == name)?;
            Some(c.local?.then(actor))
        }
        let mut w = MatchWorld::default();
        let mut rng = Rng::new(seed);
        // Level actors and their spawners.
        for layout in layouts {
            for a in &layout.actors {
                let Some(t) = a.transform else { continue };
                let owner = w.actors.len();
                w.actors.push(MatchActor { class: a.class.clone(), transform: t, from_level: true, spawned_by: None, kind: None });
                for c in &a.components {
                    if !classes.is_child_of(&c.kind, "SCActorSpawnerComponent") {
                        continue;
                    }
                    w.spawners.push(MatchSpawner {
                        owner,
                        component: c.name.clone(),
                        // Components from the actor's Blueprint carry no transform in the level file: place them
                        // from the class description (actor space) instead of at the actor's origin.
                        world: c.world.or_else(|| class_component_world(classes, &a.class, &c.name, &t)).unwrap_or(t),
                        actor_list: c.actor_list(),
                        spawn_on_initialize: c.spawn_on_initialize(),
                        spawned: None,
                    });
                }
            }
        }
        // "Spawn on initialize" spawners, including those of the actors they spawn.
        let mut next = 0;
        while next < w.spawners.len() {
            let s = &w.spawners[next];
            if s.spawn_on_initialize && s.spawned.is_none() && !s.actor_list.is_empty() {
                let class = s.actor_list[rng.index(s.actor_list.len())].clone();
                let at = s.world;
                w.spawn(classes, next, &class, at, None);
            }
            next += 1;
        }

        // Match-start spawning.
        let mut sw = SpawnWorld::default();
        for s in &w.spawners {
            sw.spawners.push(Spawner {
                actor_list: s.actor_list.clone(),
                owner_pos: w.actors[s.owner].transform.t,
                spawned: s.spawned.map(|a| w.actors[a].class.clone()),
            });
        }
        for (i, s) in w.spawners.iter().enumerate() {
            let owner = &w.actors[s.owner].class;
            if s.component == "ActorSpawner" && classes.is_child_of(owner, "SCVehicleSpawner") {
                sw.vehicle_spawners.push(i);
            }
            if s.component == "ActorSpawner" && classes.is_child_of(owner, "SCRepairPartSpawner") {
                sw.part_spawners.push(i);
            }
        }
        for actor in &w.actors {
            if classes.is_child_of(&actor.class, "SCPhoneJunctionBox") {
                sw.phone_boxes.push((actor.transform.t, actor.class.clone()));
            }
        }
        for (a, actor) in w.actors.iter().enumerate() {
            if !classes.is_child_of(&actor.class, "SCCabinet") {
                continue;
            }
            let slot = |name: &str| w.spawners.iter().position(|s| s.owner == a && s.component == name);
            let slots: Vec<usize> = ["ItemSpawner", "ItemSpawner2"].into_iter().filter_map(slot).collect();
            if !slots.is_empty() {
                sw.cabinets.push(Cabinet { slots, pos: actor.transform.t });
            }
        }
        for s in spawn_match(&mut sw, settings, classes, &mut rng) {
            let mut at = w.spawners[s.spawner].world;
            // Items placed in a cabinet use their own resting rotation (`CabinetRotation`) at the attach point.
            let in_cabinet = classes.is_child_of(&w.actors[w.spawners[s.spawner].owner].class, "SCCabinet");
            if in_cabinet {
                let r = classes.prop(&s.class, "CabinetRotation");
                let q = rotator_to_quat(
                    r["Pitch"].as_f64().unwrap_or(0.0) as f32,
                    r["Yaw"].as_f64().unwrap_or(0.0) as f32,
                    r["Roll"].as_f64().unwrap_or(0.0) as f32,
                );
                at = Xform { r: q, ..Xform::IDENTITY }.then(&at);
            }
            w.spawn(classes, s.spawner, &s.class, at, Some(s.kind));
        }
        w
    }

    /// Spawns `class` from spawner `by` at `at` (the spawner's world transform), then runs the new actor's own
    /// "spawn on initialize" spawners later in the queue.
    fn spawn(&mut self, classes: &ClassDb, by: usize, class: &str, at: Xform, kind: Option<SpawnKind>) -> usize {
        let root = classes.root_relative(class).then(&at);
        let id = self.actors.len();
        self.actors.push(MatchActor { class: class.to_string(), transform: root, from_level: false, spawned_by: Some(by), kind });
        self.spawners[by].spawned = Some(id);
        if let Some(info) = classes.get(class) {
            for c in &info.components {
                if !classes.is_child_of(&c.kind, "SCActorSpawnerComponent") {
                    continue;
                }
                self.spawners.push(MatchSpawner {
                    owner: id,
                    component: c.name.clone(),
                    world: c.local.unwrap_or(Xform::IDENTITY).then(&root),
                    actor_list: c.actor_list(),
                    spawn_on_initialize: c.spawn_on_initialize(),
                    spawned: None,
                });
            }
        }
        id
    }

    /// Actors created at runtime (not part of the level placements), with their classes.
    pub fn spawned_actors(&self) -> impl Iterator<Item = (usize, &MatchActor)> {
        self.actors.iter().enumerate().filter(|(_, a)| !a.from_level)
    }

    /// The cabinet actor holding this actor, if it was spawned into one.
    pub fn cabinet_of(&self, actor: usize, classes: &ClassDb) -> Option<usize> {
        let by = self.actors[actor].spawned_by?;
        let owner = self.spawners[by].owner;
        classes.is_child_of(&self.actors[owner].class, "SCCabinet").then_some(owner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotator_matches_unreal() {
        // Yaw 90: +X turns to +Y.
        let q = rotator_to_quat(0.0, 90.0, 0.0);
        let v = crate::level::quat_rotate(q, [1.0, 0.0, 0.0]);
        assert!((v[0]).abs() < 1e-5 && (v[1] - 1.0).abs() < 1e-5, "{v:?}");
        // Pitch 90: +X turns to +Z.
        let q = rotator_to_quat(90.0, 0.0, 0.0);
        let v = crate::level::quat_rotate(q, [1.0, 0.0, 0.0]);
        assert!((v[2] - 1.0).abs() < 1e-5, "{v:?}");
    }
}

/// Builds real matches when `F13_EXPORT` points at an export (skipped otherwise).
#[cfg(test)]
mod export_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn real_match_has_cars_parts_and_items() {
        let Some(root) = std::env::var_os("F13_EXPORT").map(PathBuf::from) else { return };
        let load = |n: &str| LevelLayout::load(&root.join("levels").join(format!("{n}.json"))).unwrap();
        let classes = ClassDb::load(&root.join("classes.json")).unwrap();
        for layout in 1..=4 {
            let layouts = vec![load("CrystalLake_Small"), load(&format!("CLsm_Design_SL{layout}"))];
            let settings = spawn_settings(&MatchWorld::world_settings(&layouts));
            for seed in 0..5 {
                let w = MatchWorld::build(&layouts, &classes, &settings, seed);
                let count = |k: SpawnKind| w.actors.iter().filter(|a| a.kind == Some(k)).count();
                let classes_of = |k: SpawnKind| {
                    let mut v: Vec<&str> = w.actors.iter().filter(|a| a.kind == Some(k)).map(|a| a.class.as_str()).collect();
                    v.sort();
                    v
                };
                println!(
                    "SL{layout} seed {seed}: {} actors, {} spawned at init; vehicles {:?}; parts {:?}; {} large, {} small",
                    w.actors.len(),
                    w.actors.iter().filter(|a| !a.from_level && a.kind.is_none()).count(),
                    classes_of(SpawnKind::Vehicle),
                    classes_of(SpawnKind::RepairPart),
                    count(SpawnKind::LargeItem),
                    count(SpawnKind::SmallItem),
                );
                // Items land at their own spawners, not stacked at a building's origin.
                let mut stacks = std::collections::HashMap::new();
                for a in w.actors.iter().filter(|a| matches!(a.kind, Some(SpawnKind::LargeItem | SpawnKind::SmallItem))) {
                    *stacks.entry(a.transform.t.map(|v| v.round() as i64)).or_insert(0) += 1;
                }
                let worst = stacks.iter().max_by_key(|(_, n)| **n).map(|(p, n)| (*p, *n)).unwrap();
                assert!(worst.1 <= 3, "SL{layout} seed {seed}: {} items at {:?}", worst.1, worst.0);
                assert!(count(SpawnKind::Vehicle) >= 1);
                let parts = classes_of(SpawnKind::RepairPart);
                for p in ["BP_CarBattery_C", "CarGasCan_C", "CarKeys_C", "PhoneBoxFuse_C"] {
                    assert!(parts.contains(&p), "SL{layout} seed {seed}: no {p}");
                }
            }
        }
    }
}
