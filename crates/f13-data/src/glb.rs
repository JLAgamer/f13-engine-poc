//! A minimal reader for the export's binary glTF meshes: a primitive's raw vertex streams, every UV channel included
//! (Bevy's glTF loader keeps TEXCOORD_0 and _1 only; mesh-particle materials read more, as M_Firecracker_Debris_01's
//! chunk index in the third).

use std::path::Path;

/// One triangle-list primitive: positions, normals and tangents as stored, each TEXCOORD_n in order, indices.
#[derive(Debug, Default, Clone)]
pub struct Primitive {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub uvs: Vec<Vec<[f32; 2]>>,
    pub indices: Vec<u32>,
}

fn accessor(json: &serde_json::Value, bin: &[u8], index: usize) -> Option<(Vec<f32>, usize)> {
    let a = &json["accessors"][index];
    let view = &json["bufferViews"][a["bufferView"].as_u64()? as usize];
    let comps = match a["type"].as_str()? {
        "SCALAR" => 1,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" => 4,
        _ => return None,
    };
    let (size, read): (usize, fn(&[u8]) -> f32) = match a["componentType"].as_u64()? {
        5126 => (4, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        5125 => (4, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32),
        5123 => (2, |b| u16::from_le_bytes([b[0], b[1]]) as f32),
        5121 => (1, |b| b[0] as f32),
        _ => return None,
    };
    let count = a["count"].as_u64()? as usize;
    let start = view["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
    let stride = view["byteStride"].as_u64().map_or(comps * size, |s| s as usize);
    let mut out = Vec::with_capacity(count * comps);
    for i in 0..count {
        for c in 0..comps {
            let o = start + i * stride + c * size;
            out.push(read(bin.get(o..o + size)?));
        }
    }
    Some((out, comps))
}

/// The first mesh's primitives of a .glb file (None when it isn't one this reader handles).
pub fn read_primitives(path: &Path) -> Option<Vec<Primitive>> {
    let b = std::fs::read(path).ok()?;
    if b.get(0..4)? != b"glTF" {
        return None;
    }
    let json_len = u32::from_le_bytes(b.get(12..16)?.try_into().ok()?) as usize;
    let json: serde_json::Value = serde_json::from_slice(b.get(20..20 + json_len)?).ok()?;
    let bin = b.get(20 + json_len + 8..)?;
    let mut out = vec![];
    for p in json["meshes"][0]["primitives"].as_array()? {
        let attrs = &p["attributes"];
        let get = |name: &str| attrs[name].as_u64().and_then(|i| accessor(&json, bin, i as usize));
        let vec3 = |v: Vec<f32>| v.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect::<Vec<_>>();
        let mut prim = Primitive { positions: vec3(get("POSITION")?.0), ..Default::default() };
        if let Some((n, _)) = get("NORMAL") {
            prim.normals = vec3(n);
        }
        if let Some((t, _)) = get("TANGENT") {
            prim.tangents = t.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
        }
        let mut i = 0;
        while let Some((uv, _)) = get(&format!("TEXCOORD_{i}")) {
            prim.uvs.push(uv.chunks_exact(2).map(|c| [c[0], c[1]]).collect());
            i += 1;
        }
        prim.indices = match p["indices"].as_u64() {
            Some(i) => accessor(&json, bin, i as usize)?.0.into_iter().map(|v| v as u32).collect(),
            None => (0..prim.positions.len() as u32).collect(),
        };
        out.push(prim);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    /// The grass impact's shard mesh, when `F13_EXPORT` points at an export (skipped otherwise): its third UV set's x
    /// numbers the four shards (0.5..3.5).
    #[test]
    fn shard_mesh_chunks() {
        let Some(root) = std::env::var_os("F13_EXPORT").map(std::path::PathBuf::from) else { return };
        let path = root.join("SummerCamp/Content/Effects/GDC_2016/ParticleMesh/S_Door_Cabin_Door_01_Shards_01.glb");
        if !path.exists() {
            return;
        }
        let prims = super::read_primitives(&path).expect("reads");
        let p = &prims[0];
        assert_eq!(p.positions.len(), 124);
        assert_eq!(p.indices.len(), 510);
        assert_eq!(p.normals.len(), 124);
        let mut counts = [0; 5];
        for uv in &p.uvs[2] {
            counts[uv[0].ceil() as usize] += 1;
        }
        assert_eq!(counts, [0, 32, 28, 28, 36]);
    }
}
