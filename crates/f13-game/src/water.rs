//! Water volumes (`water.json` from f13-export: the map's volumes with bWaterVolume and their brush boxes; their
//! transforms from the level layout). A character is in water when its location (the capsule's centre) is inside one,
//! as Unreal's IsInWater asks the physics volume at the pawn's location.

use bevy::prelude::*;
use serde::Deserialize;

pub struct WaterPlugin;

impl Plugin for WaterPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Water>().add_systems(Startup, load);
    }
}

/// The water volumes as world boxes (Bevy space, metres; the map's volumes are unrotated).
#[derive(Resource, Default)]
pub struct Water {
    pub boxes: Vec<(Vec3, Vec3)>,
}

impl Water {
    /// The surface (the top of the volume) over a point inside water.
    pub fn surface_at(&self, p: Vec3) -> Option<f32> {
        self.boxes.iter().find(|(lo, hi)| p.cmpge(*lo).all() && p.cmple(*hi).all()).map(|(_, hi)| hi.y)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Volume {
    name: String,
    boxes: Vec<UeBox>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UeBox {
    min: UeVec,
    max: UeVec,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "PascalCase")]
struct UeVec {
    x: f32,
    y: f32,
    z: f32,
}

fn load(export: Res<crate::ExportRoot>, level: Res<crate::Level>, mut water: ResMut<Water>) {
    let path = export.0.join("water.json");
    let Some(volumes) = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Vec<Volume>>(&t).ok()) else {
        warn!("{}: missing (f13-export water): no swimming", path.display());
        return;
    };
    for v in volumes {
        let Some(xf) = level.layouts.iter().flat_map(|l| &l.actors).find(|a| a.name == v.name).and_then(|a| a.transform.as_ref()).map(crate::to_transform) else {
            continue;
        };
        for b in v.boxes {
            // Unreal (x, y, z) cm -> Bevy (x, z, y) m, then the actor's transform; the corners' bounds.
            let corner = |x: f32, y: f32, z: f32| xf.transform_point(Vec3::new(x, z, y) * 0.01);
            let (mut lo, mut hi) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
            for x in [b.min.x, b.max.x] {
                for y in [b.min.y, b.max.y] {
                    for z in [b.min.z, b.max.z] {
                        let c = corner(x, y, z);
                        lo = lo.min(c);
                        hi = hi.max(c);
                    }
                }
            }
            info!("water: {} from {lo} to {hi}", v.name);
            water.boxes.push((lo, hi));
        }
    }
}
