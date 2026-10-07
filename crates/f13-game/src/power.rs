//! Power: the map's fuse boxes (SCFuseBox: PowerGridFuseBox_C) and the grids they feed (SCPowerGridVolume: a box,
//! its PoweredArea). The powered lights (Powered{Spot,Point}LightComponent) inside a grid's box go out while its fuse
//! box is destroyed, and the fuse box's red PowerOutPointLight comes on; out, they don't light the counselor (her
//! darkness fear). Jason destroys a fuse box (the player's E; its DestroyScoreEvent), a counselor repairs it (hold E,
//! HoldTimeLimit 15 s, with pips; its RepairScoreEvent).
//!
//! Not ported: the power curves (lights flicker off and on), the blackout fear event, the fuse box's light
//! material, its sounds, the computer Jason cutting the power.

use bevy::prelude::*;
use f13_data::classes::ClassDb;
use f13_data::level::Xform;

pub struct PowerPlugin {
    /// Every fuse box starts destroyed (testing: `--cut-power`, the power bot).
    pub cut: bool,
}

/// [`PowerPlugin::cut`].
#[derive(Resource)]
struct CutPower(bool);

impl Plugin for PowerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CutPower(self.cut))
            .init_resource::<PowerGrids>().add_systems(Startup, spawn_fuse_boxes).add_systems(Update, apply_power);
    }
}

#[derive(Clone, Debug)]
pub struct Grid {
    pub name: String,
    /// The box (Unreal cm): its transform and half extent.
    xf: Xform,
    half: Vec3,
    pub on: bool,
}

/// The map's power grids.
#[derive(Resource, Default, Clone, Debug)]
pub struct PowerGrids {
    pub grids: Vec<Grid>,
}

impl PowerGrids {
    /// The level's SCPowerGridVolumes (their PoweredArea's BoxExtent, 32 by default, times the actor's scale).
    pub fn from_level(level: &crate::Level) -> Self {
        let grids = level
            .layouts
            .iter()
            .flat_map(|l| &l.actors)
            .filter(|a| a.class == "SCPowerGridVolume")
            .filter_map(|a| {
                let xf = a.transform?;
                let ext = a.components.iter().find(|c| c.kind == "BoxComponent").map(|c| &c.props["BoxExtent"]);
                let e = |k: &str| ext.and_then(|e| e[k].as_f64()).map_or(32.0, |v| v as f32);
                let half = Vec3::new(e("X") * xf.s[0].abs(), e("Y") * xf.s[1].abs(), e("Z") * xf.s[2].abs());
                Some(Grid { name: a.name.clone(), xf, half, on: true })
            })
            .collect();
        PowerGrids { grids }
    }

    /// The grid whose box holds `p` (Unreal cm).
    pub fn grid_at(&self, p: Vec3) -> Option<usize> {
        self.grids.iter().position(|g| {
            let local = Quat::from_array(g.xf.r).inverse() * (p - Vec3::from_array(g.xf.t));
            local.abs().cmple(g.half).all()
        })
    }

    pub fn on(&self, grid: usize) -> bool {
        self.grids.get(grid).is_none_or(|g| g.on)
    }
}

/// A fuse box: the grid it feeds, whether it's destroyed, where it's worked on (Bevy space) and how long a repair
/// takes.
#[derive(Component)]
pub struct FuseBox {
    pub class: String,
    pub grid: Option<usize>,
    pub destroyed: bool,
    pub interact: Vec3,
    /// cm.
    pub reach: f32,
    pub hold_time: f32,
    pub has_pips: bool,
}

/// A powered light (in grid `.0`).
#[derive(Component)]
pub struct Powered(pub usize);

/// A fuse box's light for when its power is out (grid `.0`).
#[derive(Component)]
pub struct PowerOutLight(pub usize);

fn spawn_fuse_boxes(mut commands: Commands, level: Res<crate::Level>, export: Res<crate::ExportRoot>, mut grids: ResMut<PowerGrids>, cut: Res<CutPower>) {
    *grids = PowerGrids::from_level(&level);
    let classes = ClassDb::load(&export.0.join("classes.json")).unwrap_or_default();
    let mut n = 0;
    for a in level.layouts.iter().flat_map(|l| &l.actors) {
        if !classes.is_child_of(&a.class, "SCFuseBox") {
            continue;
        }
        let Some(t) = a.transform else { continue };
        // Its PowerGrid: "SCPowerGridVolume'<actor name>'".
        let grid = a.props["PowerGrid"].as_str().and_then(|s| s.split('\'').nth(1)).and_then(|name| grids.grids.iter().position(|g| g.name == name));
        let info = classes.get(&a.class);
        let interact = info.and_then(|c| c.components.iter().find(|c| c.name == "Interactable"));
        let local = interact.and_then(|c| c.local).unwrap_or(Xform::IDENTITY);
        let at = crate::to_transform(&local.then(&t)).translation;
        let props = interact.map(|c| &c.props);
        let destroyed = cut.0 || a.props["bStartDestroyed"].as_bool().unwrap_or(false);
        commands.spawn((
            FuseBox {
                class: a.class.clone(),
                grid,
                destroyed,
                interact: at,
                reach: props.and_then(|p| p["DistanceLimit"].as_f64()).map_or(150.0, |v| v as f32),
                hold_time: props.and_then(|p| p["HoldTimeLimit"].as_f64()).map_or(15.0, |v| v as f32),
                has_pips: props.is_some_and(|p| p["Pips"].as_array().is_some_and(|p| !p.is_empty())),
            },
            crate::to_transform(&t),
            Name::new(a.name.clone()),
        ));
        if let (Some(g), true) = (grid, destroyed) {
            grids.grids[g].on = false;
        }
        n += 1;
        debug!("power: {} feeds {:?}", a.name, grid.map(|g| &grids.grids[g].name));
    }
    info!("power: {} grids, {n} fuse boxes", grids.grids.len());
}

/// Lights follow their grid.
fn apply_power(grids: Res<PowerGrids>, mut powered: Query<(&Powered, &mut Visibility), Without<PowerOutLight>>, mut out: Query<(&PowerOutLight, &mut Visibility), Without<Powered>>) {
    if !grids.is_changed() {
        return;
    }
    for (p, mut v) in &mut powered {
        v.set_if_neq(if grids.on(p.0) { Visibility::Inherited } else { Visibility::Hidden });
    }
    for (p, mut v) in &mut out {
        v.set_if_neq(if grids.on(p.0) { Visibility::Hidden } else { Visibility::Inherited });
    }
}

/// Cuts or restores a fuse box's power.
pub fn set_destroyed(fuse: &mut FuseBox, destroyed: bool, grids: &mut PowerGrids) {
    fuse.destroyed = destroyed;
    if let Some(g) = fuse.grid {
        grids.grids[g].on = !destroyed;
        info!("power: {} {}", grids.grids[g].name, if destroyed { "out" } else { "back on" });
    }
}
