//! Level layouts written by `tools/f13-export` (`levels/<map>.json`): where every mesh, landscape and gameplay
//! actor sits, in Unreal space. Use [`Xform::to_gltf_space`] to convert to the glTF/Bevy convention.

use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LevelLayout {
    pub format: u32,
    pub map: String,
    pub levels: Vec<String>,
    pub placements: Vec<Placement>,
    pub actors: Vec<ActorInfo>,
    pub landscapes: Vec<LandscapeInfo>,
    /// The sky sphere's material parameters (its MaterialInstanceConstant's scalars, and vectors as [r, g, b]).
    #[serde(default)]
    pub sky: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Placement {
    pub level: String,
    pub component: String,
    /// Unreal object path of the static mesh, e.g. `/Game/Props/X/Y.Y`.
    pub mesh: String,
    /// glTF file relative to the export root.
    pub file: String,
    pub hidden: bool,
    #[serde(default)]
    pub materials: Vec<Option<String>>,
    /// Material overrides: the mesh's own slot material name -> the material used instead.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub material_map: std::collections::HashMap<String, String>,
    pub transform: Option<Xform>,
    pub instances: Option<Vec<Xform>>,
    /// Blocks counselors (its collision profile); its shapes are in collision.json. Exports before collision: true.
    #[serde(default = "yes")]
    pub blocks_pawn: bool,
    /// Casts shadows (the component's CastShadow).
    #[serde(default = "yes")]
    pub cast_shadow: bool,
    /// Not drawn past this distance from the camera, cm (foliage InstanceEndCullDistance, else the max draw
    /// distance); 0: always drawn.
    #[serde(default)]
    pub cull_distance: f32,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ActorInfo {
    pub level: String,
    pub name: String,
    pub class: String,
    pub transform: Option<Xform>,
    /// The actor's gameplay properties (scalars, enums, structs, class names), for actors other than plain meshes.
    #[serde(default)]
    pub props: serde_json::Value,
    /// Its non-mesh components (spawners, volumes, ...) with world transforms.
    #[serde(default, deserialize_with = "null_as_default")]
    pub components: Vec<ComponentInfo>,
    /// BlockingVolumes: convex hulls as world-space point sets (Unreal cm, flat x, y, z).
    #[serde(default, deserialize_with = "null_as_default")]
    pub hulls: Vec<Vec<f32>>,
    /// NavMeshBoundsVolumes: the box their brush covers (Unreal cm: min x, y, z, max x, y, z).
    #[serde(default)]
    pub bounds: Option<[f32; 6]>,
}

/// A component of a placed actor (`world` set) or of a class (`local`: actor space, relative to the root).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ComponentInfo {
    pub name: String,
    #[serde(rename = "Type")]
    pub kind: String,
    #[serde(default)]
    pub parent: Option<String>,
    /// glTF file (relative to the export root) for mesh components.
    #[serde(default)]
    pub mesh: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    /// Material overrides: the mesh's own slot material name -> the material used instead.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub material_map: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub local: Option<Xform>,
    #[serde(default)]
    pub world: Option<Xform>,
    /// Skeletal mesh components: the mesh's sockets in component space.
    #[serde(default)]
    pub sockets: std::collections::HashMap<String, Xform>,
    /// This is the class's root component; `root_relative` is applied on top of the spawn transform.
    #[serde(default)]
    pub root: bool,
    #[serde(default)]
    pub root_relative: Option<Xform>,
    #[serde(default)]
    pub props: serde_json::Value,
    /// Static mesh components: blocks counselors (default for exports without the flag).
    #[serde(default = "yes")]
    pub blocks_pawn: bool,
    /// Box, sphere and capsule components that block counselors: a convex point set in component space (Unreal cm).
    #[serde(default)]
    pub hull: Option<Vec<f32>>,
}

impl ComponentInfo {
    /// Classes an `SCActorSpawnerComponent` can spawn.
    pub fn actor_list(&self) -> Vec<String> {
        str_list(&self.props["ActorList"])
    }

    pub fn spawn_on_initialize(&self) -> bool {
        self.props["bSpawnOnInitialize"].as_bool().unwrap_or(false)
    }
}

/// The exporter writes `null` for absent lists.
pub(crate) fn null_as_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

pub fn str_list(v: &serde_json::Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LandscapeInfo {
    pub level: String,
    pub actor: String,
    pub file: String,
}

/// Unreal transform: centimetres, X forward, Y right, Z up (left-handed); rotation quaternion (x, y, z, w).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Xform {
    pub t: [f32; 3],
    pub r: [f32; 4],
    pub s: [f32; 3],
}

/// A transform in glTF/Bevy space: metres, Y up, right-handed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GltfXform {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl Xform {
    pub const IDENTITY: Xform = Xform { t: [0.0; 3], r: [0.0, 0.0, 0.0, 1.0], s: [1.0; 3] };

    /// `self * parent` in Unreal's convention: apply `self` (a child's relative transform), then `parent`.
    /// Scale is composed per axis (no shear), as Unreal does for components.
    pub fn then(&self, parent: &Xform) -> Xform {
        let scaled = [self.t[0] * parent.s[0], self.t[1] * parent.s[1], self.t[2] * parent.s[2]];
        let rotated = quat_rotate(parent.r, scaled);
        Xform {
            t: [rotated[0] + parent.t[0], rotated[1] + parent.t[1], rotated[2] + parent.t[2]],
            r: quat_mul(parent.r, self.r),
            s: [self.s[0] * parent.s[0], self.s[1] * parent.s[1], self.s[2] * parent.s[2]],
        }
    }

    /// Rotates a direction (no translation or scale).
    pub fn rotate(&self, v: [f32; 3]) -> [f32; 3] {
        quat_rotate(self.r, v)
    }

    /// Same convention CUE4Parse uses when writing glTF meshes: swap Y and Z (which also flips handedness)
    /// and scale centimetres to metres.
    pub fn to_gltf_space(&self) -> GltfXform {
        let [x, y, z] = self.t;
        let [qx, qy, qz, qw] = self.r;
        let [sx, sy, sz] = self.s;
        GltfXform {
            translation: [x * 0.01, z * 0.01, y * 0.01],
            // Reflecting the axes negates the rotation angle: (x, z, y, -w) == (-x, -z, -y, w).
            rotation: [-qx, -qz, -qy, qw],
            scale: [sx, sz, sy],
        }
    }
}

pub fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

pub fn quat_rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = q;
    let u = [x, y, z];
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let t = cross(u, v).map(|c| 2.0 * c);
    let c = cross(u, t);
    [v[0] + w * t[0] + c[0], v[1] + w * t[1] + c[1], v[2] + w * t[2] + c[2]]
}

impl LevelLayout {
    pub fn load(path: &Path) -> Result<Self, crate::LoadError> {
        let text = std::fs::read_to_string(path).map_err(crate::LoadError::Io)?;
        serde_json::from_str(&text).map_err(crate::LoadError::Json)
    }

    /// Gameplay actors of one class (e.g. `SCVehicleSpawner`, `SCCounselorPlayerStart`).
    pub fn actors_of<'a>(&'a self, class: &'a str) -> impl Iterator<Item = &'a ActorInfo> + 'a {
        self.actors.iter().filter(move |a| a.class == class)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::quat_rotate as rotate;

    /// Rotating then converting must equal converting then rotating with the converted quaternion.
    #[test]
    fn conversion_commutes_with_rotation() {
        let h = (0.3f32).sin();
        let q = { let n = (0.2f32 * 0.2 + 0.5 * 0.5 + 0.7 * 0.7).sqrt(); [0.2 / n * h, 0.5 / n * h, 0.7 / n * h, (0.3f32).cos()] };
        let v = [100.0, -50.0, 25.0];
        let xf = Xform { t: [0.0; 3], r: q, s: [1.0; 3] };
        let g = xf.to_gltf_space();
        let ue_rotated = rotate(q, v);
        let expect = [ue_rotated[0], ue_rotated[2], ue_rotated[1]];
        let got = rotate(g.rotation, [v[0], v[2], v[1]]);
        for i in 0..3 {
            assert!((expect[i] - got[i]).abs() < 1e-3, "{expect:?} vs {got:?}");
        }
    }

    /// A child 100 cm in front of a parent turned 90 degrees about Z ends up 100 cm along the parent's Y.
    #[test]
    fn composition_applies_child_then_parent() {
        let h = std::f32::consts::FRAC_PI_4;
        let parent = Xform { t: [10.0, 0.0, 0.0], r: [0.0, 0.0, h.sin(), h.cos()], s: [2.0; 3] };
        let child = Xform { t: [100.0, 0.0, 0.0], ..Xform::IDENTITY };
        let w = child.then(&parent);
        assert!((w.t[0] - 10.0).abs() < 1e-3 && (w.t[1] - 200.0).abs() < 1e-3, "{:?}", w.t);
        assert_eq!(w.s, [2.0; 3]);
    }

    #[test]
    fn translation_and_scale() {
        let g = Xform { t: [100.0, 200.0, 300.0], r: [0.0, 0.0, 0.0, 1.0], s: [1.0, 2.0, 3.0] }.to_gltf_space();
        assert_eq!(g.translation, [1.0, 3.0, 2.0]);
        assert_eq!(g.scale, [1.0, 3.0, 2.0]);
    }
}

/// `null` (what the exporter writes for "none") as an empty map.
/// A JSON null (the exporter's empty map or list) as the empty value.
pub(crate) fn null_as_empty<'de, D: serde::Deserializer<'de>, T: Deserialize<'de> + Default>(d: D) -> Result<T, D::Error> {
    Ok(Option::deserialize(d)?.unwrap_or_default())
}
