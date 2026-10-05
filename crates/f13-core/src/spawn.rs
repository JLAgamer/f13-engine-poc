//! Match-start item and vehicle spawning, as the game mode does it after the level has loaded (and after every
//! spawner flagged "spawn on initialize" has already spawned its actor):
//!
//! 1. vehicles: one random class per `VehiclesToSpawn` entry, each at a random vehicle spawner far enough from the
//!    vehicles already placed;
//! 2. repair parts: for every vehicle, each part its repair components require, at a random cabinet (small parts,
//!    e.g. keys) or part spawner (large parts, e.g. the gas can) within range of that vehicle, spread apart;
//! 3. large items: a random count of each class, at random free spawners that list it;
//! 4. small items: a random count of each class, in random cabinets that list it, spread apart.
//!
//! Distances are Unreal units (cm). The world is described by plain data; class relationships come from
//! [`ClassQuery`], so this module has no engine or data-format dependency.

/// The game's `FMath::RandRange`-style draws use `rand()`; this is a small deterministic replacement (PCG32) so a
/// match can be reproduced from its seed.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut r = Self { state: seed.wrapping_add(0x853c_49e6_748f_ea9b) };
        r.next_u32();
        r
    }

    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Uniform in [0, 1).
    pub fn frand(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / (1u32 << 24) as f32)
    }

    /// The game's random index: `min((int)(n * FRand()), n - 1)`; 0 when `n == 0`.
    pub fn index(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        ((n as f32 * self.frand()) as usize).min(n - 1)
    }

    /// `Min + RandRange(0, Max - Min)`, inclusive; `Min` when the range is empty.
    pub fn count(&mut self, min: i32, max: i32) -> i32 {
        let span = max - min;
        if span + 1 < 1 {
            return min;
        }
        min + ((span + 1) as f32 * self.frand()).min(span as f32) as i32
    }
}

/// Class hierarchy and defaults the spawner needs.
pub trait ClassQuery {
    /// `class` is `parent` or derives from it.
    fn is_child_of(&self, class: &str, parent: &str) -> bool;
    /// The item class is carried in the hand (`bIsLarge`) rather than the pocket.
    fn is_large(&self, class: &str) -> bool;
    /// Part classes a vehicle's repair components require (gas, battery, keys...).
    fn required_parts(&self, vehicle_class: &str) -> Vec<String>;
}

/// An `SCActorSpawnerComponent`: spawns one actor from its list.
#[derive(Clone, Debug)]
pub struct Spawner {
    pub actor_list: Vec<String>,
    /// Location of the owning actor (distance checks use actor locations).
    pub owner_pos: [f32; 3],
    pub spawned: Option<String>,
}

impl Spawner {
    /// `CanSpawn`: some entry of the list is `class` or derives from it.
    pub fn can_spawn(&self, class: &str, q: &impl ClassQuery) -> bool {
        self.actor_list.iter().any(|a| q.is_child_of(a, class))
    }

    /// `SpawnActor(filter)`: a random entry (among those deriving from `filter`, when given). Returns the class.
    fn spawn(&mut self, filter: Option<&str>, q: &impl ClassQuery, rng: &mut Rng) -> Option<String> {
        if self.spawned.is_some() || self.actor_list.is_empty() {
            return None;
        }
        let candidates: Vec<&String> = match filter {
            Some(f) => self.actor_list.iter().filter(|a| q.is_child_of(a, f)).collect(),
            None => self.actor_list.iter().collect(),
        };
        if candidates.is_empty() {
            return None;
        }
        let class = candidates[rng.index(candidates.len())].clone();
        self.spawned = Some(class.clone());
        Some(class)
    }
}

/// An `SCCabinet` (drawer, desk...): one item slot per item spawner (double cabinets have two).
#[derive(Clone, Debug)]
pub struct Cabinet {
    /// Indices into [`SpawnWorld::spawners`]; the first one decides which classes the cabinet accepts.
    pub slots: Vec<usize>,
    pub pos: [f32; 3],
}

#[derive(Clone, Debug, Default)]
pub struct SpawnWorld {
    pub spawners: Vec<Spawner>,
    /// `SCVehicleSpawner` actors (their actor spawner).
    pub vehicle_spawners: Vec<usize>,
    /// `SCRepairPartSpawner` actors (their actor spawner).
    pub part_spawners: Vec<usize>,
    pub cabinets: Vec<Cabinet>,
}

#[derive(Clone, Debug, Default)]
pub struct SpawnSettings {
    /// One entry per vehicle to spawn: the classes to pick from.
    pub vehicles: Vec<Vec<String>>,
    pub large_items: Vec<CountedClass>,
    pub small_items: Vec<CountedClass>,
    pub min_vehicle_spawn_distance: f32,
    pub min_vehicle_part_spawn_distance: f32,
    pub max_car_part_distance: f32,
}

#[derive(Clone, Debug)]
pub struct CountedClass {
    pub class: String,
    pub min: i32,
    pub max: i32,
    /// Small items only: keep copies of this class this far apart while there are spare cabinets.
    pub min_spawn_distance: f32,
}

/// What spawned where: `spawner` indexes [`SpawnWorld::spawners`].
#[derive(Clone, Debug, PartialEq)]
pub struct Spawned {
    pub spawner: usize,
    pub class: String,
    pub kind: SpawnKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnKind {
    Vehicle,
    RepairPart,
    LargeItem,
    SmallItem,
}

fn dist_sq(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

impl SpawnWorld {
    fn cabinet_full(&self, c: &Cabinet) -> bool {
        c.slots.iter().all(|&s| self.spawners[s].spawned.is_some())
    }

    /// `SCCabinet::SpawnItem`: fills the first free slot that can hold the class.
    fn cabinet_spawn(&mut self, cabinet: usize, class: &str, q: &impl ClassQuery, rng: &mut Rng) -> Option<usize> {
        let slots = self.cabinets[cabinet].slots.clone();
        for s in slots {
            if self.spawners[s].spawned.is_none() && self.spawners[s].can_spawn(class, q) {
                self.spawners[s].spawn(Some(class), q, rng)?;
                return Some(s);
            }
        }
        None
    }

    fn cabinet_accepts(&self, c: &Cabinet, class: &str, q: &impl ClassQuery) -> bool {
        c.slots.first().is_some_and(|&s| self.spawners[s].can_spawn(class, q))
    }
}

/// Runs the match-start spawning. The large-item pass may use every spawner in the world that has not spawned yet
/// and is not a cabinet slot.
pub fn spawn_match(world: &mut SpawnWorld, s: &SpawnSettings, q: &impl ClassQuery, rng: &mut Rng) -> Vec<Spawned> {
    let mut out = Vec::new();
    let vehicles = spawn_vehicles(world, s, q, rng, &mut out);
    spawn_repair_parts(world, s, q, rng, &vehicles, &mut out);
    spawn_large_items(world, s, q, rng, &mut out);
    spawn_small_items(world, s, q, rng, &mut out);
    out
}

/// Returns (vehicle spawner index, class) for each vehicle placed.
fn spawn_vehicles(world: &mut SpawnWorld, s: &SpawnSettings, q: &impl ClassQuery, rng: &mut Rng, out: &mut Vec<Spawned>) -> Vec<(usize, String)> {
    // Pick one class per entry (duplicates within an entry count once).
    let mut chosen = Vec::new();
    for entry in &s.vehicles {
        let mut unique: Vec<&String> = Vec::new();
        for c in entry {
            if !unique.contains(&c) {
                unique.push(c);
            }
        }
        if !unique.is_empty() {
            chosen.push(unique[rng.index(unique.len())].clone());
        }
    }
    let mut placed: Vec<(usize, String)> = Vec::new();
    for class in chosen {
        let mut candidates: Vec<usize> =
            world.vehicle_spawners.iter().copied().filter(|&i| world.spawners[i].can_spawn(&class, q)).collect();
        while !candidates.is_empty() {
            let k = rng.index(candidates.len());
            let sp = candidates.remove(k);
            let pos = world.spawners[sp].owner_pos;
            let far_enough = placed.iter().all(|&(other, _)| {
                dist_sq(world.spawners[other].owner_pos, pos) >= s.min_vehicle_spawn_distance.powi(2)
            });
            if !far_enough {
                continue;
            }
            if let Some(c) = world.spawners[sp].spawn(Some(&class), q, rng) {
                out.push(Spawned { spawner: sp, class: c.clone(), kind: SpawnKind::Vehicle });
                placed.push((sp, c));
                break;
            }
        }
    }
    placed
}

fn spawn_repair_parts(
    world: &mut SpawnWorld,
    s: &SpawnSettings,
    q: &impl ClassQuery,
    rng: &mut Rng,
    vehicles: &[(usize, String)],
    out: &mut Vec<Spawned>,
) {
    let max_sq = s.max_car_part_distance.powi(2);
    let mut placed_parts: Vec<[f32; 3]> = Vec::new();
    for (vsp, vclass) in vehicles {
        let vpos = world.spawners[*vsp].owner_pos;
        for part in q.required_parts(vclass) {
            let large = q.is_large(&part);
            // Candidate spawn points within range of this vehicle.
            let mut candidates: Vec<usize> = if large {
                world.part_spawners.iter().copied()
                    .filter(|&i| world.spawners[i].spawned.is_none() && world.spawners[i].can_spawn(&part, q))
                    .filter(|&i| s.max_car_part_distance == 0.0 || dist_sq(world.spawners[i].owner_pos, vpos) < max_sq)
                    .collect()
            } else {
                (0..world.cabinets.len())
                    .filter(|&c| world.cabinet_accepts(&world.cabinets[c], &part, q) && !world.cabinet_full(&world.cabinets[c]))
                    .filter(|&c| s.max_car_part_distance == 0.0 || dist_sq(world.cabinets[c].pos, vpos) < max_sq)
                    .collect()
            };
            while !candidates.is_empty() {
                let k = rng.index(candidates.len());
                let cand = candidates[k];
                let pos = if large { world.spawners[cand].owner_pos } else { world.cabinets[cand].pos };
                let spread = s.min_vehicle_part_spawn_distance <= 0.0
                    || placed_parts.iter().all(|&p| dist_sq(p, pos) >= s.min_vehicle_part_spawn_distance.powi(2));
                if !spread {
                    candidates.remove(k);
                    continue;
                }
                let spawned = if large {
                    world.spawners[cand].spawn(Some(&part), q, rng).map(|_| cand)
                } else {
                    world.cabinet_spawn(cand, &part, q, rng)
                };
                candidates.remove(k);
                if let Some(sp) = spawned {
                    out.push(Spawned { spawner: sp, class: part.clone(), kind: SpawnKind::RepairPart });
                    placed_parts.push(pos);
                    break;
                }
            }
        }
    }
}

fn spawn_large_items(world: &mut SpawnWorld, s: &SpawnSettings, q: &impl ClassQuery, rng: &mut Rng, out: &mut Vec<Spawned>) {
    let in_cabinet: std::collections::HashSet<usize> = world.cabinets.iter().flat_map(|c| c.slots.iter().copied()).collect();
    // The candidate lists are built once, before any large item spawns.
    let lists: Vec<Vec<usize>> = s
        .large_items
        .iter()
        .map(|item| {
            (0..world.spawners.len())
                .filter(|i| !in_cabinet.contains(i))
                .filter(|&i| world.spawners[i].spawned.is_none() && world.spawners[i].can_spawn(&item.class, q))
                .collect()
        })
        .collect();
    for (item, mut list) in s.large_items.iter().zip(lists) {
        let target = rng.count(item.min, item.max);
        let mut n = 0;
        while n < target && !list.is_empty() {
            let k = rng.index(list.len());
            let sp = list.remove(k);
            // A spawner already used by an earlier class fails; the game then just tries another one.
            if let Some(c) = world.spawners[sp].spawn(Some(&item.class), q, rng) {
                out.push(Spawned { spawner: sp, class: c, kind: SpawnKind::LargeItem });
                n += 1;
            }
        }
    }
}

fn spawn_small_items(world: &mut SpawnWorld, s: &SpawnSettings, q: &impl ClassQuery, rng: &mut Rng, out: &mut Vec<Spawned>) {
    let lists: Vec<Vec<usize>> = s
        .small_items
        .iter()
        .map(|item| (0..world.cabinets.len()).filter(|&c| world.cabinet_accepts(&world.cabinets[c], &item.class, q)).collect())
        .collect();
    for (item, mut list) in s.small_items.iter().zip(lists) {
        let target = rng.count(item.min, item.max);
        let mut placed: Vec<[f32; 3]> = Vec::new();
        while (placed.len() as i32) < target && !list.is_empty() {
            let k = rng.index(list.len());
            let cab = list[k];
            if world.cabinet_full(&world.cabinets[cab]) {
                list.remove(k);
                continue;
            }
            let remaining = target - placed.len() as i32;
            let pos = world.cabinets[cab].pos;
            if remaining < list.len() as i32
                && item.min_spawn_distance > 0.0
                && placed.iter().any(|&p| dist_sq(p, pos) < item.min_spawn_distance.powi(2))
            {
                list.remove(k);
                continue;
            }
            match world.cabinet_spawn(cab, &item.class, q, rng) {
                Some(sp) => {
                    out.push(Spawned { spawner: sp, class: item.class.clone(), kind: SpawnKind::SmallItem });
                    placed.push(pos);
                    if world.cabinet_full(&world.cabinets[cab]) {
                        list.remove(k);
                    }
                }
                None => {
                    list.remove(k);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Q;
    impl ClassQuery for Q {
        fn is_child_of(&self, class: &str, parent: &str) -> bool {
            class == parent || (parent == "Item" && class.starts_with("Item"))
        }
        fn is_large(&self, class: &str) -> bool {
            class == "Gas"
        }
        fn required_parts(&self, _: &str) -> Vec<String> {
            vec!["Gas".into(), "Keys".into()]
        }
    }

    fn spawner(list: &[&str], pos: [f32; 3]) -> Spawner {
        Spawner { actor_list: list.iter().map(|s| s.to_string()).collect(), owner_pos: pos, spawned: None }
    }

    #[test]
    fn rng_index_and_count_stay_in_range() {
        let mut r = Rng::new(7);
        for n in 1..20 {
            for _ in 0..200 {
                assert!(r.index(n) < n);
            }
        }
        for _ in 0..500 {
            let c = r.count(3, 5);
            assert!((3..=5).contains(&c));
        }
        assert_eq!(r.count(4, 4), 4);
        assert_eq!(r.count(4, 2), 4);
    }

    fn world() -> SpawnWorld {
        let mut w = SpawnWorld::default();
        // Two vehicle spawners 100 m apart, one 10 m from the first.
        for pos in [[0.0, 0.0, 0.0], [10_000.0, 0.0, 0.0], [1_000.0, 0.0, 0.0]] {
            w.vehicle_spawners.push(w.spawners.len());
            w.spawners.push(spawner(&["CarA", "CarB"], pos));
        }
        // Gas spawners: one near the cars, one too far.
        for pos in [[2_000.0, 0.0, 0.0], [90_000.0, 0.0, 0.0]] {
            w.part_spawners.push(w.spawners.len());
            w.spawners.push(spawner(&["Gas"], pos));
        }
        // Cabinets that take keys and items.
        for x in 0..6 {
            let slot = w.spawners.len();
            w.spawners.push(spawner(&["Keys", "ItemMap", "ItemKnife"], [x as f32 * 500.0, 3_000.0, 0.0]));
            w.cabinets.push(Cabinet { slots: vec![slot], pos: [x as f32 * 500.0, 3_000.0, 0.0] });
        }
        // Weapon spawners.
        for x in 0..4 {
            w.spawners.push(spawner(&["Bat", "Axe"], [0.0, x as f32 * 100.0, 0.0]));
        }
        w
    }

    fn settings() -> SpawnSettings {
        SpawnSettings {
            vehicles: vec![vec!["CarA".into()], vec!["CarB".into()]],
            large_items: vec![CountedClass { class: "Bat".into(), min: 2, max: 2, min_spawn_distance: 0.0 }],
            small_items: vec![CountedClass { class: "ItemMap".into(), min: 3, max: 3, min_spawn_distance: 1_000.0 }],
            min_vehicle_spawn_distance: 8_000.0,
            min_vehicle_part_spawn_distance: 3_000.0,
            max_car_part_distance: 16_500.0,
        }
    }

    #[test]
    fn vehicles_are_spread_out_and_parts_stay_in_range() {
        for seed in 0..50 {
            let mut w = world();
            let out = spawn_match(&mut w, &settings(), &Q, &mut Rng::new(seed));
            let cars: Vec<_> = out.iter().filter(|s| s.kind == SpawnKind::Vehicle).collect();
            assert_eq!(cars.len(), 2, "seed {seed}");
            let (a, b) = (w.spawners[cars[0].spawner].owner_pos, w.spawners[cars[1].spawner].owner_pos);
            assert!(dist_sq(a, b) >= 8_000f32.powi(2));
            // Never the far gas spawner.
            assert!(out.iter().all(|s| s.spawner != 4), "seed {seed}");
            // Exactly one gas can (the second car's gas would be within 30 m of the first), keys in cabinets.
            let gas = out.iter().filter(|s| s.class == "Gas").count();
            assert_eq!(gas, 1, "seed {seed}");
            let keys: Vec<_> = out.iter().filter(|s| s.class == "Keys").collect();
            assert!(!keys.is_empty());
            for k in &keys {
                assert!(w.cabinets.iter().any(|c| c.slots.contains(&k.spawner)));
            }
        }
    }

    #[test]
    fn items_fill_counts_and_cabinets_hold_one_each() {
        let mut w = world();
        let out = spawn_match(&mut w, &settings(), &Q, &mut Rng::new(3));
        assert_eq!(out.iter().filter(|s| s.class == "Bat").count(), 2);
        assert_eq!(out.iter().filter(|s| s.class == "ItemMap").count(), 3);
        let mut used = std::collections::HashSet::new();
        for s in &out {
            assert!(used.insert(s.spawner), "spawner {} used twice", s.spawner);
        }
    }
}
