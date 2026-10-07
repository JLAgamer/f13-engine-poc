//! The exporter's `collision.json` (simple collision per static mesh) and triangle reading from exported glTF
//! binaries (terrain, and meshes that collide with their render triangles).

use crate::level::Xform;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MeshShapes {
    /// Convex point sets in mesh space (Unreal cm, flat x, y, z).
    #[serde(default)]
    pub hulls: Vec<Vec<f32>>,
    /// The mesh collides with its render triangles (CTF_UseComplexAsSimple).
    #[serde(default)]
    pub complex_as_simple: bool,
    /// Its physical surface type (EPhysicalSurface; 0 the default).
    #[serde(default)]
    pub surface: u8,
}

/// glTF path -> shapes.
pub fn load(path: &Path) -> Result<HashMap<String, MeshShapes>, crate::LoadError> {
    let text = std::fs::read_to_string(path).map_err(crate::LoadError::Io)?;
    let map: HashMap<String, MeshShapes> = serde_json::from_str(&text).map_err(crate::LoadError::Json)?;
    Ok(map.into_iter().map(|(k, v)| (k.to_lowercase(), v)).collect())
}

/// A flat Unreal point list (cm) as glTF-space points (metres, Y up).
pub fn gltf_points(flat: &[f32]) -> Vec<[f32; 3]> {
    flat.chunks_exact(3).map(|p| Xform { t: [p[0], p[1], p[2]], ..Xform::IDENTITY }.to_gltf_space().translation).collect()
}

/// Every triangle of a glTF binary's meshes in the file's own space (node transforms applied, scenes ignored).
pub fn glb_triangles(path: &Path) -> Result<Vec<[[f32; 3]; 3]>, crate::LoadError> {
    let bytes = std::fs::read(path).map_err(crate::LoadError::Io)?;
    parse_glb(&bytes, &mut |_, _| {}).ok_or_else(|| crate::LoadError::Invalid(format!("{}: not a glTF binary this reader handles", path.display())))
}

/// Every vertex of a glTF binary's triangle meshes: its position (node transforms applied) and first UV set.
pub fn glb_vertices(path: &Path) -> Result<Vec<([f32; 3], [f32; 2])>, crate::LoadError> {
    let bytes = std::fs::read(path).map_err(crate::LoadError::Io)?;
    let mut out = Vec::new();
    parse_glb(&bytes, &mut |p, uv| {
        if let Some(uv) = uv {
            out.push((p, uv));
        }
    })
    .ok_or_else(|| crate::LoadError::Invalid(format!("{}: not a glTF binary this reader handles", path.display())))?;
    Ok(out)
}

fn parse_glb(b: &[u8], on_vertex: &mut dyn FnMut([f32; 3], Option<[f32; 2]>)) -> Option<Vec<[[f32; 3]; 3]>> {
    let u32_at = |o: usize| Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize);
    if b.get(0..4)? != b"glTF" {
        return None;
    }
    let json_len = u32_at(12)?;
    let json: serde_json::Value = serde_json::from_slice(b.get(20..20 + json_len)?).ok()?;
    let bin_at = 20 + json_len;
    let bin_len = u32_at(bin_at)?;
    let bin = b.get(bin_at + 8..bin_at + 8 + bin_len)?;

    let accessor = |i: usize| -> Option<(Vec<f32>, usize)> {
        let a = &json["accessors"][i];
        let view = &json["bufferViews"][a["bufferView"].as_u64()? as usize];
        let count = a["count"].as_u64()? as usize;
        let comps = match a["type"].as_str()? {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            _ => return None,
        };
        let ctype = a["componentType"].as_u64()?;
        let size = match ctype {
            5126 | 5125 => 4,
            5123 => 2,
            5121 => 1,
            _ => return None,
        };
        let start = view["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let stride = view["byteStride"].as_u64().map_or(comps * size, |s| s as usize);
        let mut out = Vec::with_capacity(count * comps);
        for e in 0..count {
            for c in 0..comps {
                let o = start + e * stride + c * size;
                let raw = bin.get(o..o + size)?;
                out.push(match ctype {
                    5126 => f32::from_le_bytes(raw.try_into().ok()?),
                    5125 => u32::from_le_bytes(raw.try_into().ok()?) as f32,
                    5123 => u16::from_le_bytes(raw.try_into().ok()?) as f32,
                    _ => raw[0] as f32,
                });
            }
        }
        Some((out, comps))
    };

    // Node transforms: each mesh node's world matrix (column-major 4x4), walking from the roots.
    let nodes = json["nodes"].as_array().cloned().unwrap_or_default();
    let mut parent = vec![None; nodes.len()];
    for (i, n) in nodes.iter().enumerate() {
        for c in n["children"].as_array().into_iter().flatten() {
            if let Some(c) = c.as_u64() {
                parent[c as usize] = Some(i);
            }
        }
    }
    let local = |n: &serde_json::Value| -> [[f32; 4]; 4] {
        let f = |k: &str, d: &[f32]| -> Vec<f32> {
            n[k].as_array().map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_else(|| d.to_vec())
        };
        if n["matrix"].is_array() {
            let m = f("matrix", &[]);
            return [[m[0], m[1], m[2], m[3]], [m[4], m[5], m[6], m[7]], [m[8], m[9], m[10], m[11]], [m[12], m[13], m[14], m[15]]];
        }
        let t = f("translation", &[0.0, 0.0, 0.0]);
        let r = f("rotation", &[0.0, 0.0, 0.0, 1.0]);
        let s = f("scale", &[1.0, 1.0, 1.0]);
        let (x, y, z, w) = (r[0], r[1], r[2], r[3]);
        let rot = [
            [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w)],
            [2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w)],
            [2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y)],
        ];
        [
            [rot[0][0] * s[0], rot[0][1] * s[0], rot[0][2] * s[0], 0.0],
            [rot[1][0] * s[1], rot[1][1] * s[1], rot[1][2] * s[1], 0.0],
            [rot[2][0] * s[2], rot[2][1] * s[2], rot[2][2] * s[2], 0.0],
            [t[0], t[1], t[2], 1.0],
        ]
    };
    let mul = |a: [[f32; 4]; 4], b: [[f32; 4]; 4]| {
        let mut m = [[0.0; 4]; 4];
        for (c, col) in m.iter_mut().enumerate() {
            for (r, v) in col.iter_mut().enumerate() {
                *v = (0..4).map(|k| a[k][r] * b[c][k]).sum();
            }
        }
        m
    };
    let world = |mut i: usize| {
        let mut m = local(&nodes[i]);
        while let Some(p) = parent[i] {
            m = mul(local(&nodes[p]), m);
            i = p;
        }
        m
    };
    let apply = |m: &[[f32; 4]; 4], p: [f32; 3]| [0, 1, 2].map(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r]);

    let mut tris = Vec::new();
    for (ni, n) in nodes.iter().enumerate() {
        let Some(mesh) = n["mesh"].as_u64() else { continue };
        let m = world(ni);
        for prim in json["meshes"][mesh as usize]["primitives"].as_array().into_iter().flatten() {
            if prim["mode"].as_u64().unwrap_or(4) != 4 {
                continue;
            }
            let (pos, 3) = accessor(prim["attributes"]["POSITION"].as_u64()? as usize)? else { return None };
            let verts: Vec<[f32; 3]> = pos.chunks_exact(3).map(|p| apply(&m, [p[0], p[1], p[2]])).collect();
            let uvs: Option<Vec<[f32; 2]>> = prim["attributes"]["TEXCOORD_0"]
                .as_u64()
                .and_then(|i| accessor(i as usize))
                .filter(|(_, comps)| *comps == 2)
                .map(|(d, _)| d.chunks_exact(2).map(|c| [c[0], c[1]]).collect());
            for (i, v) in verts.iter().enumerate() {
                on_vertex(*v, uvs.as_ref().and_then(|u| u.get(i).copied()));
            }
            let indices: Vec<usize> = match prim["indices"].as_u64() {
                Some(i) => accessor(i as usize)?.0.into_iter().map(|v| v as usize).collect(),
                None => (0..verts.len()).collect(),
            };
            for t in indices.chunks_exact(3) {
                tris.push([*verts.get(t[0])?, *verts.get(t[1])?, *verts.get(t[2])?]);
            }
        }
    }
    Some(tris)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal glb: one node (translated by +1 on X) with one indexed triangle.
    fn glb() -> Vec<u8> {
        let mut bin = Vec::new();
        for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0] {
            bin.extend(v.to_le_bytes());
        }
        for i in [0u16, 2, 1, 0] {
            bin.extend(i.to_le_bytes()); // 3 indices + padding
        }
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "nodes": [{"mesh": 0, "translation": [1.0, 0.0, 0.0]}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"},
                {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}
            ],
            "bufferViews": [{"buffer": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 6}],
            "buffers": [{"byteLength": bin.len()}]
        });
        let mut j = serde_json::to_vec(&json).unwrap();
        while j.len() % 4 != 0 {
            j.push(b' ');
        }
        let mut b = b"glTF".to_vec();
        b.extend(2u32.to_le_bytes());
        b.extend(((12 + 8 + j.len() + 8 + bin.len()) as u32).to_le_bytes());
        b.extend((j.len() as u32).to_le_bytes());
        b.extend(b"JSON");
        b.extend(j);
        b.extend((bin.len() as u32).to_le_bytes());
        b.extend(b"BIN\0");
        b.extend(bin);
        b
    }

    #[test]
    fn reads_triangles_with_node_transforms() {
        let tris = parse_glb(&glb(), &mut |_, _| {}).unwrap();
        assert_eq!(tris, vec![[[1.0, 0.0, 0.0], [1.0, 0.0, 1.0], [2.0, 0.0, 0.0]]]);
    }

    #[test]
    fn unreal_points_become_gltf_metres() {
        assert_eq!(gltf_points(&[100.0, 200.0, 300.0]), vec![[1.0, 3.0, 2.0]]);
    }
}
