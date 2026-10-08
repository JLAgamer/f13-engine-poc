//! Jason's fade (his materials' 0_AlphaDelta), as his tick (FUN_14040d900) runs it for every Jason: a timer (0x1424,
//! 1 from the constructor) falls at 1 a second to 0 while EnableTeleportEffect is set (Shift sets it with
//! bIsShiftActive, FUN_1404050d0) and rises back to 1 after; FUN_140406a20 sets 0_AlphaDelta = 1 - it on every
//! material of his Mesh and JasonSkeletalMask. The materials turn it into their opacity mask's cutoff
//! (materials.rs `fade_cutoff`), so he fades out as Shift starts and back in after. His weapon (opaque) stays.

use bevy::gltf::GltfMaterialName;
use bevy::prelude::*;

pub struct KillerFadePlugin;

impl Plugin for KillerFadePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, fade);
    }
}

/// The timer (1: all there).
#[derive(Component)]
struct Fade(f32);

#[allow(clippy::type_complexity)]
fn fade(
    mut commands: Commands,
    time: Res<Time>,
    index: Option<Res<crate::materials::MaterialIndex>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut jasons: Query<(Entity, &crate::jason::Jason, Option<&mut Fade>)>,
    rigs: Query<(Entity, &crate::character::Rig)>,
    children: Query<&Children>,
    meshes: Query<(&GltfMaterialName, &MeshMaterial3d<StandardMaterial>)>,
) {
    let Some(index) = index else { return };
    for (e, j, fade) in &mut jasons {
        let Some(mut fade) = fade else {
            commands.entity(e).insert(Fade(1.0));
            continue;
        };
        let dt = time.delta_secs();
        let t = if j.shifting() { (fade.0 - dt).max(0.0) } else { (fade.0 + dt).min(1.0) };
        fade.0 = t;
        let alpha_delta = 1.0 - t;
        for (rig_entity, rig) in &rigs {
            if !matches!(rig.model, crate::character::Model::Killer) {
                continue;
            }
            // Every frame: his materials replace the glTF's once they're in (materials.rs); a cutoff only changes the
            // material when it differs.
            for m in children.iter_descendants(rig_entity) {
                let Ok((name, handle)) = meshes.get(m) else { continue };
                let Some(cutoff) = index.fade_cutoff(&name.0, alpha_delta) else { continue };
                if let Some(mut mat) = materials.get_mut(&handle.0) {
                    if mat.alpha_mode != AlphaMode::Mask(cutoff) {
                        mat.alpha_mode = AlphaMode::Mask(cutoff);
                    }
                }
            }
        }
    }
}
