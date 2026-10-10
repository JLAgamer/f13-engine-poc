//! The world's blocking geometry for character movement, from the export: every placed mesh's simple collision
//! (collision.json) where its collision profile blocks counselors, BlockingVolumes, and the terrain. Built once at
//! startup into a static grid; class actors (doors, windows, cars, furniture) carry `Collider`s that follow them.

use bevy::prelude::*;
use f13_core::collision::{convex_hull, Tri, TriGrid, TriList, TriSource};
use f13_data::collision::{gltf_points, MeshShapes};
use std::collections::{HashMap, HashSet};

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
    /// What blocks the camera channel (the spring arms' probe): the placements, volumes and terrain whose collision
    /// blocks Camera.
    pub camera_grid: std::sync::Arc<TriGrid>,
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

/// A [`Collider`] whose component also blocks the camera channel (the spring arms' probe).
#[derive(Component, Clone, Copy)]
pub struct CameraBlocker;

/// The level's interactable actors' own triangles in the static grid (fuse boxes, phone boxes, phones), by actor name:
/// an interactable's occlusion trace ignores its own actor.
#[derive(Resource, Default)]
pub struct ActorTris(pub HashMap<String, HashSet<TriKey>>);

/// A triangle's identity (its corners' bits).
pub type TriKey = [u32; 9];

pub fn tri_key(t: &Tri) -> TriKey {
    let [a, b, c] = [t.a, t.b, t.c].map(|v| v.to_array().map(f32::to_bits));
    [a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2]]
}

/// A triangle source without some triangles.
pub struct Excluding<'a, S: ?Sized>(pub &'a S, pub Option<&'a HashSet<TriKey>>);

impl<S: TriSource + ?Sized> TriSource for Excluding<'_, S> {
    fn for_each_in(&self, min: Vec3, max: Vec3, f: &mut dyn FnMut(&Tri)) {
        match self.1 {
            Some(skip) => self.0.for_each_in(min, max, &mut |t| {
                if !skip.contains(&tri_key(t)) {
                    f(t)
                }
            }),
            None => self.0.for_each_in(min, max, f),
        }
    }
}

/// A collider that blocks her but not traces on WorldDynamic (some roofs' invisible blockers, the cars' body boxes):
/// interactables' occlusion traces pass it.
#[derive(Component)]
pub struct NoWorldDynamic;

/// A dynamic collider that doesn't block traces on Visibility (its component's response).
#[derive(Component)]
pub struct NoVisibility;

/// The level placements' triangles that block her but not traces on Visibility (window glass and more).
#[derive(Resource, Default)]
pub struct NotVisibility(pub HashSet<TriKey>);

/// The level placements' triangles that block her but not traces on WorldDynamic.
#[derive(Resource, Default)]
pub struct NotWorldDynamic(pub HashSet<TriKey>);

/// The level's triangles that block her but not the Vehicle channel (a car's body passes them: the police
/// blockers' volumes, some props).
#[derive(Resource, Default)]
pub struct NotVehicle(pub HashSet<TriKey>);

/// A dynamic collider that doesn't block the Vehicle channel.
#[derive(Component)]
pub struct NoVehicle;

/// An actor's shapes that overlap queries on WorldDynamic find (convex hulls in the actor's space, as placed).
#[derive(Component, Clone)]
pub struct QueryShapes(pub Vec<Vec<Tri>>);

impl QueryShapes {
    /// Whether a sphere overlapping queries on WorldDynamic (an overlap of the actor at `at`) finds the actor.
    pub fn overlaps_sphere(&self, at: &GlobalTransform, centre: Vec3, radius: f32) -> bool {
        let m = at.affine();
        self.0.iter().any(|h| f13_core::collision::sphere_overlaps_hull(&transform_tris(h, &m), centre, radius))
    }
}

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

pub(crate) fn transform_tris(tris: &[Tri], m: &bevy::math::Affine3A) -> Vec<Tri> {
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
    let mut camera_grid = TriGrid::new(4.0);
    let (mut placed, mut volumes) = (0usize, 0usize);
    // The level's interactable actors (their placements' triangles are kept by actor too).
    let classes = f13_data::classes::ClassDb::load(&export.0.join("classes.json")).ok();
    let interactables: HashSet<&str> = level
        .layouts
        .iter()
        .flat_map(|l| &l.actors)
        .filter(|a| classes.as_ref().and_then(|c| Some((c, c.get(&a.class)?))).is_some_and(|(c, info)| info.components.iter().any(|x| c.is_child_of(&x.kind, "ILLInteractableComponent"))))
        .map(|a| a.name.as_str())
        .collect();
    let mut actor_tris = ActorTris::default();
    let mut not_visibility = NotVisibility::default();
    let mut not_world_dynamic = NotWorldDynamic::default();
    let mut not_vehicle = NotVehicle::default();
    for layout in &level.layouts {
        for p in &layout.placements {
            if !p.blocks_pawn && !p.blocks_camera {
                continue;
            }
            // "<package>:PersistentLevel.<actor>.<component>".
            let actor = p.component.split("PersistentLevel.").nth(1).and_then(|s| s.split('.').next()).filter(|a| interactables.contains(a));
            let tris = local(&p.file);
            // A spline mesh's collision bent along its spline: its convex elements re-hulled, else its triangles.
            let tris = match (&p.spline, shapes_ref.get(&p.file.to_lowercase())) {
                (Some(s), Some(sh)) if !sh.complex_as_simple => crate::spline_meshes::bend_hulls(s, &sh.hulls, sh.surface, &export.0.join(&p.file)),
                (Some(s), _) => crate::spline_meshes::bend_tris(s, &tris),
                (None, _) => tris,
            };
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
                    if let Some(a) = actor {
                        actor_tris.0.entry(a.to_string()).or_default().insert(tri_key(&t));
                    }
                    if p.blocks_pawn && !p.blocks_visibility {
                        not_visibility.0.insert(tri_key(&t));
                    }
                    if p.blocks_pawn && !p.blocks_world_dynamic {
                        not_world_dynamic.0.insert(tri_key(&t));
                    }
                    if p.blocks_pawn && !p.blocks_vehicle {
                        not_vehicle.0.insert(tri_key(&t));
                    }
                    if p.blocks_pawn {
                        grid.insert(t);
                    }
                    if p.blocks_camera {
                        camera_grid.insert(t);
                    }
                }
                placed += 1;
            }
        }
        for a in &layout.actors {
            // The police's blockers come down when they arrive (police.rs): colliders of their own.
            if crate::police::is_blocker(&a.name) {
                let tris: Vec<Tri> = a.hulls.iter().flat_map(|h| hull_from_unreal(h)).collect();
                let e = commands.spawn((Collider(std::sync::Arc::new(tris)), Transform::IDENTITY, Visibility::default(), crate::police::Blocker, Name::new(a.name.clone()))).id();
                if !a.hulls_block_vehicle {
                    commands.entity(e).insert(NoVehicle);
                }
                continue;
            }
            // Only the BlockingVolumes' hulls collide (the AudioVolumes' are for audio_volumes.rs).
            if a.class != "BlockingVolume" {
                continue;
            }
            for h in &a.hulls {
                for t in hull_from_unreal(h) {
                    if a.hulls_block_pawn && !a.hulls_block_vehicle {
                        not_vehicle.0.insert(tri_key(&t));
                    }
                    if a.hulls_block_pawn {
                        grid.insert(t);
                    }
                    if a.hulls_block_camera {
                        camera_grid.insert(t);
                    }
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
                            camera_grid.insert(t);
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
    commands.insert_resource(WorldCollision { grid: std::sync::Arc::new(grid), camera_grid: std::sync::Arc::new(camera_grid), shapes });
    commands.insert_resource(actor_tris);
    info!("collision: {} level triangles pass Visibility traces, {} WorldDynamic ones", not_visibility.0.len(), not_world_dynamic.0.len());
    commands.insert_resource(not_visibility);
    commands.insert_resource(not_world_dynamic);
    commands.insert_resource(not_vehicle);
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


