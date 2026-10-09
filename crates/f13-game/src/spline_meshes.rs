//! Spline meshes (the landscape's roads, the guard rails): each placement's own copy of its mesh bent along its
//! spline (f13-core spline_mesh, in the component's Unreal space) once its scene is in; their collision triangles
//! are bent the same way (collision.rs).

use bevy::camera::primitives::Aabb;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use f13_data::level::SplineMesh;

pub struct SplineMeshPlugin;

impl Plugin for SplineMeshPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, bend);
    }
}

/// A spline mesh placement whose meshes aren't bent yet.
#[derive(Component)]
pub struct Bend(pub SplineMesh);

/// glTF space (m, Y up) to Unreal's (cm, Z up), and back (the exporter's swap of Y and Z).
fn to_ue(p: [f32; 3], scale: f32) -> [f32; 3] {
    [p[0] * scale, p[2] * scale, p[1] * scale]
}

/// The mesh's extent along the spline's forward axis (cm), over all its points (glTF space).
fn extent<'a>(spline: &SplineMesh, points: impl Iterator<Item = &'a [f32; 3]>) -> (f32, f32) {
    points.map(|p| to_ue(*p, 100.0)[spline.forward_axis.min(2)]).fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)))
}

/// A point and normal (glTF space) bent along the spline.
fn bend_point(params: &f13_core::spline_mesh::SplineParams, p: [f32; 3], n: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let (q, m) = params.deform(to_ue(p, 100.0), to_ue(n, 1.0));
    (to_ue(q, 0.01), to_ue(m, 1.0))
}

/// Collision triangles (glTF space, the mesh's own) bent along the spline.
pub(crate) fn bend_tris(spline: &SplineMesh, tris: &[f13_core::collision::Tri]) -> Vec<f13_core::collision::Tri> {
    let pts: Vec<[f32; 3]> = tris.iter().flat_map(|t| [t.a, t.b, t.c]).map(|v| v.to_array()).collect();
    let (lo, hi) = extent(spline, pts.iter());
    let params = spline.params(lo, hi);
    let b = |v: Vec3| Vec3::from_array(bend_point(&params, v.to_array(), [0.0, 1.0, 0.0]).0);
    tris.iter().filter_map(|t| f13_core::collision::Tri::new(b(t.a), b(t.b), b(t.c)).map(|n| n.with_surface(t.surface))).collect()
}

fn bend(mut commands: Commands, roots: Query<(Entity, &Bend)>, children: Query<&Children>, parts: Query<&Mesh3d>, mut meshes: ResMut<Assets<Mesh>>) {
    for (root, b) in &roots {
        let parts: Vec<(Entity, Handle<Mesh>)> = children.iter_descendants(root).filter_map(|e| parts.get(e).ok().map(|m| (e, m.0.clone()))).collect();
        // Its scene isn't in yet, or a mesh isn't.
        if parts.is_empty() || parts.iter().any(|(_, h)| meshes.get(h).is_none()) {
            continue;
        }
        let positions = |m: &Mesh| match m.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(v)) => v.clone(),
            _ => Vec::new(),
        };
        let all: Vec<[f32; 3]> = parts.iter().flat_map(|(_, h)| meshes.get(h).map(positions).unwrap_or_default()).collect();
        let (lo, hi) = extent(&b.0, all.iter());
        let params = b.0.params(lo, hi);
        for (e, h) in parts {
            let Some(mut mesh) = meshes.get(&h).cloned() else { continue };
            let normals = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
                Some(VertexAttributeValues::Float32x3(v)) => v.clone(),
                _ => Vec::new(),
            };
            let (bent, bent_normals): (Vec<[f32; 3]>, Vec<[f32; 3]>) =
                positions(&mesh).into_iter().enumerate().map(|(i, p)| bend_point(&params, p, normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]))).unzip();
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, bent);
            if !normals.is_empty() {
                mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, bent_normals);
            }
            if mesh.attribute(Mesh::ATTRIBUTE_TANGENT).is_some() {
                let _ = mesh.generate_tangents();
            }
            commands.entity(e).insert(Mesh3d(meshes.add(mesh))).remove::<Aabb>();
        }
        commands.entity(root).remove::<Bend>();
    }
}
