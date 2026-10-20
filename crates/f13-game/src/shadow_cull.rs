//! The moon's cascades leave out the shadow casters too small to matter, as the game's renderer does
//! (FGatherShadowPrimitivesPacket::FilterPrimitiveForShadows, FUN_140ddb700): for a view-dependent whole-scene shadow
//! (the directional light's cascades), a primitive within a cascade casts only when its bounds' sphere radius R is
//! at least r.Shadow.RadiusThreshold (0.01: FAutoConsoleVariableRef 0x14014f3b0 on DAT_142d55cd0; not raised by the
//! game's DefaultScalability [ShadowQuality@3]) times its distance d from the view (its bounds' origin to the view's):
//! d^2 x T^2 x the view's LOD distance factor squared (+0xad0) <= R^2. (Reflective shadow maps take
//! r.Shadow.RadiusThresholdRSM instead; the game has none.)
//!
//! f13recomp: Bevy's cascade culling (check_dir_light_mesh_visibility) fills each camera's cascades; this drops the
//! casters below the threshold afterwards. PLACEHOLDERS: the view's LOD distance factor taken as 1
//! (FSceneViewInitOptions' default; its writer not read), and a mesh's sphere radius as its box's half diagonal
//! (UE's own bounding sphere isn't exported), per Bevy mesh (a glTF primitive, where UE's primitive is the whole
//! component).

use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::CascadesVisibleEntities;
use bevy::light::SimulationLightSystems;
use bevy::prelude::*;

/// r.Shadow.RadiusThreshold.
const RADIUS_THRESHOLD: f32 = 0.01;

pub struct ShadowCullPlugin;

impl Plugin for ShadowCullPlugin {
    fn build(&self, app: &mut App) {
        // (F13_SHADOW_RADIUS_CULL=0: off, for measuring.)
        if std::env::var("F13_SHADOW_RADIUS_CULL").is_ok_and(|v| v == "0") {
            return;
        }
        app.add_systems(PostUpdate, cull_small_casters.after(SimulationLightSystems::CheckLightVisibility));
    }
}

fn cull_small_casters(mut lights: Query<&mut CascadesVisibleEntities, With<DirectionalLight>>, views: Query<&GlobalTransform>, meshes: Query<(&Aabb, &GlobalTransform)>) {
    for mut visible in &mut lights {
        for (view, cascades) in visible.entities.iter_mut() {
            let Ok(eye) = views.get(*view).map(GlobalTransform::translation) else { continue };
            for cascade in cascades.iter_mut() {
                cascade.entities.retain(|&e| {
                    let Ok((aabb, xf)) = meshes.get(e) else { return true };
                    let affine = xf.affine();
                    let radius = (affine.matrix3 * aabb.half_extents).length();
                    let centre = Vec3::from(affine.transform_point3a(aabb.center));
                    centre.distance_squared(eye) * RADIUS_THRESHOLD * RADIUS_THRESHOLD <= radius * radius
                });
            }
        }
    }
}
