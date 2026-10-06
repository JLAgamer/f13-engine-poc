//! The match's navmesh, built at load time from the world's blocking collision and the spawned furniture the way the
//! game's RecastNavMesh does (`RuntimeGeneration=Dynamic`), with its settings for Jason's agent (f13-core::recast):
//! DefaultEngine.ini `[/Script/Engine.RecastNavMesh]` CellSize 9, CellHeight 10, TileSizeUU 3000 (333 cells),
//! AgentMaxSlope 45, AgentMaxStepHeight 40, MaxSimplificationError 1.2, monotone regions; the "Killer" agent's
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

pub struct NavigationPlugin {
    pub debug: bool,
}

/// UE's longest wall edge for the navmesh (12 m, in cells): `maxEdgeLen = 1200 / CellSize`.
const MAX_EDGE_CM: f32 = 1200.0;

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
    }
}

#[derive(Resource)]
struct DebugDraw;

impl Plugin for NavigationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, build_navmesh.run_if(not(resource_exists::<Navmesh>)));
        if self.debug {
            app.insert_resource(DebugDraw).add_systems(Update, draw_navmesh);
        }
    }
}

/// The build running in the background.
#[derive(Resource)]
struct Building(bevy::tasks::Task<NavMesh>);

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
    mut settled: Local<(usize, f32)>,
) {
    if let Some(mut task) = building {
        if let Some(mesh) = bevy::tasks::block_on(bevy::tasks::poll_once(&mut task.0)) {
            commands.remove_resource::<Building>();
            commands.insert_resource(Navmesh(Arc::new(mesh)));
        }
        return;
    }
    let Some(collision) = collision else { return };
    // Wait for the spawned actors' colliders: their count must hold for a second.
    let count = dynamic.iter().count();
    if count == 0 || count != settled.0 {
        *settled = (count, time.elapsed_secs());
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
    let task = bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
        let started = std::time::Instant::now();
        let world = (&*grid, lists.as_slice());
        let (min, max) = bounds.unwrap_or_else(|| grid.bounds());
        let cfg = killer_config();
        let stats = f13_core::recast::BuildStats::default();
        let tiles = f13_core::recast::build(&world, &cfg, min, max, &stats);
        let mesh = NavMesh::new(&tiles, Vec3::new(min.x, 0.0, min.z), cfg.tile_size(), cfg.agent_max_climb);
        let links: usize = mesh.polys.iter().map(|p| p.portals.len()).sum();
        let s = stats.seconds();
        info!(
            "navmesh: {} tiles, {} polygons, {} links, from {} colliders ({:.1} s; thread time: rasterize {:.1}, filter {:.1}, compact {:.1}, regions {:.1}, contours {:.1}, polygons {:.1})",
            tiles.len(),
            mesh.polys.len(),
            links,
            lists.len(),
            started.elapsed().as_secs_f32(),
            s[0], s[1], s[2], s[3], s[4], s[5]
        );
        // Testing: F13_NAVMESH_DUMP=<file> writes the polygons (Bevy space, metres) as JSON.
        if let Ok(path) = std::env::var("F13_NAVMESH_DUMP") {
            let polys: Vec<Vec<[f32; 3]>> = mesh.polys.iter().map(|p| p.verts.iter().map(|&v| mesh.verts[v as usize].to_array()).collect()).collect();
            match std::fs::write(&path, format!("{polys:?}")) {
                Ok(()) => info!("navmesh written to {path}"),
                Err(e) => warn!("{path}: {e}"),
            }
        }
        mesh
    });
    commands.insert_resource(Building(task));
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
