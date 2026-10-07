//! The world's blocking geometry for character movement, from the export: every placed mesh's simple collision
//! (collision.json) where its collision profile blocks counselors, BlockingVolumes, and the terrain. Built once at
//! startup into a static grid; class actors (doors, windows, cars, furniture) carry `Collider`s that follow them.

use bevy::prelude::*;
use f13_core::collision::{convex_hull, Tri, TriGrid, TriList};
use f13_data::collision::{gltf_points, MeshShapes};
use std::collections::HashMap;

pub struct CollisionPlugin;

impl Plugin for CollisionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DynamicColliders>()
            .add_systems(PreStartup, build_world)
            .add_systems(PreUpdate, refresh_dynamic);
    }
}

/// Static blocking triangles (Bevy space, metres).
#[derive(Resource)]
pub struct WorldCollision {
    pub grid: std::sync::Arc<TriGrid>,
    /// Simple collision per mesh glTF path (lowercase), for class components.
    shapes: HashMap<String, MeshShapes>,
}

impl WorldCollision {
    /// A collider for a class component's static mesh, if the mesh has simple collision.
    pub fn mesh_collider(&self, file: &str) -> Option<Collider> {
        let tris = hull_tris(self.shapes.get(&file.to_lowercase())?);
        (!tris.is_empty()).then(|| Collider(std::sync::Arc::new(tris)))
    }

    /// Everything that blocks: the static grid and the moving colliders.
    pub fn with<'a>(&'a self, dynamic: &'a DynamicColliders) -> (&'a TriGrid, &'a [TriList]) {
        (&self.grid, dynamic.lists.as_slice())
    }

    /// The static grid and the moving colliders except those `skip` names.
    pub fn without<'a>(&'a self, dynamic: &'a DynamicColliders, skip: impl Fn(Entity) -> bool) -> (&'a TriGrid, Vec<&'a TriList>) {
        (&self.grid, dynamic.iter().filter(|(e, _)| !skip(*e)).map(|(_, l)| l).collect())
    }
}

/// Blocking triangles in the entity's own space; active while the entity is visible.
#[derive(Component, Clone)]
pub struct Collider(pub std::sync::Arc<Vec<Tri>>);

/// World-space triangles of every active `Collider`.
#[derive(Resource, Default)]
pub struct DynamicColliders {
    lists: Vec<TriList>,
    entities: Vec<Entity>,
}

impl DynamicColliders {
    /// Each active collider's entity and world-space triangles.
    pub fn iter(&self) -> impl Iterator<Item = (Entity, &TriList)> {
        self.entities.iter().copied().zip(self.lists.iter())
    }
}

fn transform_tris(tris: &[Tri], m: &bevy::math::Affine3A) -> Vec<Tri> {
    let flip = m.matrix3.determinant() < 0.0;
    tris.iter()
        .filter_map(|t| {
            let (a, b, c) = (m.transform_point3(t.a), m.transform_point3(t.b), m.transform_point3(t.c));
            if flip { Tri::new(a, c, b) } else { Tri::new(a, b, c) }.map(|n| n.with_surface(t.surface))
        })
        .collect()
}

fn hull_tris(shapes: &MeshShapes) -> Vec<Tri> {
    shapes
        .hulls
        .iter()
        .flat_map(|h| convex_hull(&gltf_points(h).into_iter().map(Vec3::from_array).collect::<Vec<_>>()))
        .map(|t| t.with_surface(shapes.surface))
        .collect()
}

/// Triangles of a convex point set given in Unreal cm (component space), in glTF space.
pub fn hull_from_unreal(points: &[f32]) -> Vec<Tri> {
    convex_hull(&gltf_points(points).into_iter().map(Vec3::from_array).collect::<Vec<_>>())
}

fn build_world(mut commands: Commands, level: Res<crate::Level>, export: Res<crate::ExportRoot>) {
    let started = std::time::Instant::now();
    let shapes = match f13_data::collision::load(&export.0.join("collision.json")) {
        Ok(s) => s,
        Err(e) => {
            warn!("no collision.json in the export ({e}): only the terrain blocks; run tools/f13-export collision");
            HashMap::new()
        }
    };
    let mut meshes: HashMap<String, Vec<Tri>> = HashMap::new();
    let shapes_ref = &shapes;
    let mut local = |file: &str| -> Vec<Tri> {
        let key = file.to_lowercase();
        meshes
            .entry(key.clone())
            .or_insert_with(|| match shapes_ref.get(&key) {
                Some(s) if s.complex_as_simple => f13_data::collision::glb_triangles(&export.0.join(file))
                    .map(|t| t.iter().filter_map(|t| Tri::new(t[0].into(), t[1].into(), t[2].into()).map(|t| t.with_surface(s.surface))).collect())
                    .unwrap_or_default(),
                Some(s) => hull_tris(s),
                None => Vec::new(),
            })
            .clone()
    };
    let mut grid = TriGrid::new(4.0);
    let (mut placed, mut volumes) = (0usize, 0usize);
    for layout in &level.layouts {
        for p in &layout.placements {
            if !p.blocks_pawn {
                continue;
            }
            let tris = local(&p.file);
            if tris.is_empty() {
                continue;
            }
            let xforms: Vec<_> = match (&p.instances, &p.transform) {
                (Some(i), _) => i.iter().collect(),
                (None, Some(t)) => vec![t],
                _ => continue,
            };
            for x in xforms {
                let m = crate::to_transform(x).compute_affine();
                for t in transform_tris(&tris, &m) {
                    grid.insert(t);
                }
                placed += 1;
            }
        }
        for a in &layout.actors {
            // The police's blockers come down when they arrive (police.rs): colliders of their own.
            if crate::police::is_blocker(&a.name) {
                let tris: Vec<Tri> = a.hulls.iter().flat_map(|h| hull_from_unreal(h)).collect();
                commands.spawn((Collider(std::sync::Arc::new(tris)), Transform::IDENTITY, Visibility::default(), crate::police::Blocker, Name::new(a.name.clone())));
                continue;
            }
            for h in &a.hulls {
                for t in hull_from_unreal(h) {
                    grid.insert(t);
                }
                volumes += 1;
            }
        }
        for l in &layout.landscapes {
            let xf = layout
                .actors
                .iter()
                .find(|a| a.level == l.level && a.name == l.actor)
                .and_then(|a| a.transform.as_ref())
                .map(crate::to_transform)
                .unwrap_or_default()
                .compute_affine();
            match f13_data::collision::glb_triangles(&export.0.join(&l.file)) {
                Ok(tris) => {
                    for t in tris {
                        let [a, b, c] = t.map(|v| xf.transform_point3(Vec3::from_array(v)));
                        // Terrain is one-sided and faces up: order it so the normal points up.
                        if let Some(t) = Tri::new(a, b, c).map(|t| if t.n.y < 0.0 { Tri::new(a, c, b).unwrap() } else { t }.with_surface(Tri::TERRAIN)) {
                            grid.insert(t);
                        }
                    }
                }
                Err(e) => warn!("terrain collision: {e}"),
            }
        }
    }
    info!(
        "collision: {} triangles from {placed} placed meshes, {volumes} blocking volume hulls and the terrain ({:.1} s)",
        grid.len(),
        started.elapsed().as_secs_f32()
    );
    commands.insert_resource(WorldCollision { grid: std::sync::Arc::new(grid), shapes });
}

/// Rebuilds the world-space triangles of moved, shown or hidden colliders.
fn refresh_dynamic(
    mut dynamic: ResMut<DynamicColliders>,
    colliders: Query<(Entity, &Collider, &GlobalTransform, &InheritedVisibility)>,
    changed: Query<(), Or<(Changed<GlobalTransform>, Changed<InheritedVisibility>, Added<Collider>)>>,
    mut removed: RemovedComponents<Collider>,
) {
    let any_removed = removed.read().count() > 0;
    if !any_removed && changed.is_empty() {
        return;
    }
    let by_entity: HashMap<Entity, usize> = dynamic.entities.iter().enumerate().map(|(i, e)| (*e, i)).collect();
    let mut lists = Vec::new();
    let mut entities = Vec::new();
    for (e, c, gxf, vis) in &colliders {
        if !vis.get() {
            continue;
        }
        let list = match by_entity.get(&e) {
            Some(&i) if !changed.contains(e) => std::mem::take(&mut dynamic.lists[i]),
            _ => TriList::new(transform_tris(&c.0, &gxf.affine())),
        };
        lists.push(list);
        entities.push(e);
    }
    dynamic.lists = lists;
    dynamic.entities = entities;
}


