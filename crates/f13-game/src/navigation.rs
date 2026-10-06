//! The match's navmesh, built at load time from the world's blocking collision and the spawned furniture the way the
//! game's RecastNavMesh does (`RuntimeGeneration=Dynamic`), with its settings for Jason's agent (f13-core::recast):
//! DefaultEngine.ini `[/Script/Engine.RecastNavMesh]` CellSize 9, CellHeight 10, TileSizeUU 3000 (333 cells),
//! AgentMaxSlope 45, AgentMaxStepHeight 40, MaxSimplificationError 1.2, monotone regions, MergeRegionSize 200; the "Killer" agent's
//! AgentRadius 34 and AgentHeight 200 (cm). Built inside the level's NavMeshBoundsVolume. Door leaves are left out
//! (doorways are walkable; Jason opens doors).
//!
//! `--nav-debug` draws the polygons within 25 m of the counselor.

use bevy::prelude::*;
use f13_core::navmesh::NavMesh;
use f13_core::recast::NavConfig;
use std::sync::Arc;

/// Colliders the navmesh ignores (door leaves).
#[derive(Component)]
pub struct NotOnNavmesh;

#[derive(Resource)]
pub struct Navmesh(pub Arc<NavMesh>);

/// How the navmesh build is going, for the HUD.
#[derive(Resource, Default)]
pub struct NavmeshStatus(pub String);

pub struct NavigationPlugin {
    pub debug: bool,
}

/// UE's longest wall edge for the navmesh (12 m, in cells): `maxEdgeLen = 1200 / CellSize`.
const MAX_EDGE_CM: f32 = 1200.0;
/// DefaultEngine.ini MergeRegionSize (cm). Regions under MergeRegionSize² / CellSize cells merge (4444 cells): that
/// matches the cooked Killer navmesh's polygon density in the forest (within a few percent; (MergeRegionSize /
/// CellSize)², 493 cells, leaves about 25% more polygons in long slivers). UE's exact conversion isn't confirmed.
const MERGE_REGION_SIZE_CM: f32 = 200.0;
/// RecastNavMesh's RegionChunkSplits (engine default 2): ChunkyMonotone sweeps each tile in 2 x 2 chunks.
const REGION_CHUNK_SPLITS: i32 = 2;

pub fn killer_config() -> NavConfig {
    let cs = 0.09;
    NavConfig {
        cell_size: cs,
        cell_height: 0.10,
        agent_height: 2.0,
        agent_radius: 0.34,
        agent_max_climb: 0.40,
        agent_max_slope_deg: 45.0,
        tile_cells: (30.0 / cs) as i32,
        max_simplification_error: 1.2,
        max_edge_len: (MAX_EDGE_CM / 100.0 / cs) as i32,
        max_verts_per_poly: 6,
        merge_region_area: (MERGE_REGION_SIZE_CM.powi(2) / (cs * 100.0)) as usize,
        region_chunk_splits: REGION_CHUNK_SPLITS,
    }
}

#[derive(Resource)]
struct DebugDraw;

impl Plugin for NavigationPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(NavmeshStatus("Navmesh: waiting for the map".into()))
            .add_systems(Update, (build_navmesh.run_if(not(resource_exists::<Navmesh>)), check_reach.run_if(resource_added::<Navmesh>)));
        if self.debug {
            app.insert_resource(DebugDraw).add_systems(Update, draw_navmesh);
        }
    }
}

/// The build running in the background, its progress and when it started.
#[derive(Resource)]
struct Building(bevy::tasks::Task<NavMesh>, Arc<f13_core::recast::BuildStats>, std::time::Instant);

/// Once the props' colliders are in (and stay put for a moment), builds the navmesh on a background thread.
#[allow(clippy::too_many_arguments)]
fn build_navmesh(
    mut commands: Commands,
    collision: Option<Res<crate::collision::WorldCollision>>,
    dynamic: Res<crate::collision::DynamicColliders>,
    ignored: Query<(), With<NotOnNavmesh>>,
    time: Res<Time<Real>>,
    building: Option<ResMut<Building>>,
    level: Res<crate::Level>,
    mut status: ResMut<NavmeshStatus>,
    mut settled: Local<(usize, f32)>,
) {
    if let Some(mut task) = building {
        let (done, total) = task.1.progress();
        match bevy::tasks::block_on(bevy::tasks::poll_once(&mut task.0)) {
            Some(mesh) => {
                let islands = mesh.islands();
                status.0 = format!(
                    "Navmesh: {} polygons, {} islands (largest {}), built in {:.1} s",
                    mesh.polys.len(),
                    islands.len(),
                    islands.first().copied().unwrap_or(0),
                    task.2.elapsed().as_secs_f32()
                );
                commands.remove_resource::<Building>();
                commands.insert_resource(Navmesh(Arc::new(mesh)));
            }
            None => {
                status.0 = format!("Navmesh: building, tile {done} / {total} ({:.1} s; Jason paths on a coarse grid until then)", task.2.elapsed().as_secs_f32())
            }
        }
        return;
    }
    let Some(collision) = collision else { return };
    // Wait for the spawned actors' colliders: their count must hold for a second.
    let count = dynamic.iter().count();
    if count == 0 || count != settled.0 {
        *settled = (count, time.elapsed_secs());
        status.0 = format!("Navmesh: waiting for the props to spawn ({count} colliders)");
        return;
    }
    if time.elapsed_secs() - settled.1 < 1.0 {
        return;
    }
    // Where the game builds: its NavMeshBoundsVolume(s) (Unreal cm, z up), else everything that collides.
    let volumes: Vec<[f32; 6]> = level.layouts.iter().flat_map(|l| l.actors.iter().filter_map(|a| a.bounds)).collect();
    let bounds = (!volumes.is_empty()).then(|| {
        let to = |x: f32, y: f32, z: f32| Vec3::new(x, z, y) / 100.0;
        volumes.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), b| (lo.min(to(b[0], b[1], b[2])), hi.max(to(b[3], b[4], b[5]))))
    });
    let grid = collision.grid.clone();
    let lists: Vec<f13_core::collision::TriList> = dynamic.iter().filter(|(e, _)| !ignored.contains(*e)).map(|(_, l)| l.clone()).collect();
    let stats = Arc::new(f13_core::recast::BuildStats::default());
    let progress = stats.clone();
    let task = bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
        let started = std::time::Instant::now();
        let world = (&*grid, lists.as_slice());
        let (mut min, max) = bounds.unwrap_or_else(|| grid.bounds());
        let cfg = killer_config();
        // The tile grid starts at the world origin, as the game's (dtNavMeshParams orig 0).
        let ts = cfg.tile_size();
        min.x = (min.x / ts).floor() * ts;
        min.z = (min.z / ts).floor() * ts;
        // Testing: F13_NAV_PROBE=x,z;x,z... logs the voxel columns there.
        for at in std::env::var("F13_NAV_PROBE").unwrap_or_default().split(';').filter(|s| !s.is_empty()) {
            let v: Vec<f32> = at.split(',').filter_map(|n| n.trim().parse().ok()).collect();
            if let [x, z] = v[..] {
                info!("navmesh probe ({x}, {z}): {:?}", f13_core::recast::probe_column(&world, &cfg, Vec3::new(x, 0.0, z), min.y - 1.0, max.y + 1.0));
            }
        }
        let tiles = f13_core::recast::build(&world, &cfg, min, max, &stats);
        let mesh = NavMesh::new(&tiles, Vec3::new(min.x, 0.0, min.z), cfg.tile_size(), cfg.agent_max_climb);
        let links: usize = mesh.polys.iter().map(|p| p.portals.len()).sum();
        let s = stats.seconds();
        let islands = mesh.islands();
        info!(
            "navmesh: {} tiles, {} polygons ({} outlines failed), {} links, {} islands (largest {} polygons, then {:?}), from {} colliders ({:.1} s; thread time: rasterize {:.1}, filter {:.1}, compact {:.1}, regions {:.1}, contours {:.1}, polygons {:.1})",
            tiles.len(),
            mesh.polys.len(),
            stats.bad_contours.load(std::sync::atomic::Ordering::Relaxed),
            links,
            islands.len(),
            islands.first().copied().unwrap_or(0),
            &islands[1.min(islands.len())..5.min(islands.len())],
            lists.len(),
            started.elapsed().as_secs_f32(),
            s[0], s[1], s[2], s[3], s[4], s[5]
        );
        // Testing: F13_NAVMESH_DUMP=<file> writes the polygons (Bevy space, metres) as JSON, and <file>.islands each
        // polygon's island.
        if let Ok(path) = std::env::var("F13_NAVMESH_DUMP") {
            let polys: Vec<Vec<[f32; 3]>> = mesh.polys.iter().map(|p| p.verts.iter().map(|&v| mesh.verts[v as usize].to_array()).collect()).collect();
            let _ = std::fs::write(format!("{path}.islands"), format!("{:?}", mesh.island_of()));
            match std::fs::write(&path, format!("{polys:?}")) {
                Ok(()) => info!("navmesh written to {path}"),
                Err(e) => warn!("{path}: {e}"),
            }
        }
        mesh
    });
    status.0 = "Navmesh: building".into();
    commands.insert_resource(Building(task, progress, std::time::Instant::now()));
}

/// Once built: which of Jason's goals (the hiding spots' search spots, the cabinets, both sides of every door) the
/// navmesh can't take him to from where he stands (a log for finding holes in the mesh).
fn check_reach(
    navmesh: Res<Navmesh>,
    jason: Query<&GlobalTransform, With<crate::jason::Jason>>,
    spots: Query<(&crate::hiding::HidingSpot, &GlobalTransform)>,
    cabinets: Query<&GlobalTransform, With<crate::props::Cabinet>>,
    doors: Query<&GlobalTransform, With<crate::doors::Door>>,
) {
    let Some(from) = jason.iter().next().map(|g| g.translation()) else { return };
    let mut goals: Vec<(String, Vec3)> = Vec::new();
    for (s, g) in &spots {
        for side in &s.sides {
            goals.push((format!("{:?} search spot", s.kind), crate::hiding::HidingSpot::world(g, &side.search).translation));
        }
    }
    goals.extend(cabinets.iter().map(|g| ("cabinet".to_string(), g.translation())));
    for g in &doors {
        let fwd = g.forward().as_vec3();
        goals.push(("door, one side".into(), g.translation() + fwd));
        goals.push(("door, other side".into(), g.translation() - fwd));
    }
    let started = std::time::Instant::now();
    info!("navmesh: Jason at {from:.2}, on the mesh at {:?}", navmesh.0.snap(from, 10.0));
    let missed: Vec<String> = goals
        .iter()
        .filter(|(_, at)| !navmesh.0.find_path(from, *at, 60000).is_some_and(|(_, reached)| reached))
        .map(|(what, at)| format!("{what} at ({:.1}, {:.1}, {:.1})", at.x, at.y, at.z))
        .collect();
    info!("navmesh: Jason reaches {} of {} goals ({:.2} s){}", goals.len() - missed.len(), goals.len(), started.elapsed().as_secs_f32(), if missed.is_empty() { String::new() } else { format!("; not: {}", missed.join(", ")) });
}

fn draw_navmesh(mut gizmos: Gizmos, navmesh: Option<Res<Navmesh>>, player: Query<&Transform, With<crate::player::Player>>) {
    let (Some(nav), Ok(p)) = (navmesh, player.single()) else { return };
    let at = p.translation;
    for poly in &nav.0.polys {
        if poly.min.x > at.x + 25.0 || poly.max.x < at.x - 25.0 || poly.min.z > at.z + 25.0 || poly.max.z < at.z - 25.0 {
            continue;
        }
        let pts: Vec<Vec3> = poly.verts.iter().map(|&v| nav.0.verts[v as usize] + Vec3::Y * 0.05).collect();
        let open = poly.portals.len();
        let color = if open == 0 { Color::srgb(1.0, 0.2, 0.2) } else { Color::srgb(0.2, 0.9, 0.4) };
        gizmos.linestrip(pts.iter().copied().chain(std::iter::once(pts[0])), color);
    }
}
