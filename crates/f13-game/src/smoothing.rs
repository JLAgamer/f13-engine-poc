//! Counselors and Jason drawn between their fixed ticks. They move in FixedUpdate (64 Hz: net.rs TICK), and a frame
//! drew where the last tick left them: at 60 Hz one frame in sixteen took two ticks and, unsynced (--perf), frames
//! took none, so they jittered. (The game moves its characters every frame: f13-rs ticks them for the netcode.)
//!
//! Each tick's start and end are kept; after the fixed loop the transform shows the blend of the two by the
//! frame's place in the next tick (Time<Fixed>'s overstep), and the next tick starts from the tick's own end
//! again. A transform written outside the ticks (a teleport, a special move's spot, a network twin, turning to aim)
//! is taken as is, unblended, per part (place, rotation, scale).

use bevy::app::{FixedFirst, FixedLast, RunFixedMainLoop, RunFixedMainLoopSystems};
use bevy::prelude::*;

pub struct SmoothingPlugin;

impl Plugin for SmoothingPlugin {
    fn build(&self, app: &mut App) {
        // (F13_SMOOTH_LOG: the local counselor's drawn place each frame, for measuring; F13_SMOOTH=0: off.)
        if std::env::var("F13_SMOOTH_LOG").is_ok() {
            app.add_systems(Last, log_drawn);
        }
        if std::env::var("F13_SMOOTH").is_ok_and(|v| v == "0") {
            return;
        }
        app.add_systems(FixedFirst, tick_start)
            .add_systems(FixedLast, tick_end)
            .add_systems(RunFixedMainLoop, show.in_set(RunFixedMainLoopSystems::AfterFixedMainLoop))
            .add_systems(PostUpdate, track.before(bevy::transform::TransformSystems::Propagate));
    }
}

/// A fixed-tick mover's last tick's start and end, and the transform last shown between them.
#[derive(Component)]
pub struct Smoothed {
    start: Transform,
    end: Transform,
    shown: Transform,
}

impl Smoothed {
    fn at(xf: Transform) -> Self {
        Smoothed { start: xf, end: xf, shown: xf }
    }
}

/// Counselors and Jason get smoothed as they appear.
fn track(mut commands: Commands, new: Query<(Entity, &Transform), (Or<(With<crate::player::Player>, With<crate::jason::Jason>)>, Without<Smoothed>)>) {
    for (e, xf) in &new {
        commands.entity(e).try_insert(Smoothed::at(*xf));
    }
}

/// What was written since it was shown (outside the ticks), taken as is: per part, so turning her to aim between
/// ticks (items.rs aim) leaves her walk blended.
fn adopt(s: &mut Smoothed, xf: &Transform) {
    if xf.translation != s.shown.translation {
        (s.start.translation, s.end.translation) = (xf.translation, xf.translation);
    }
    if xf.rotation != s.shown.rotation {
        (s.start.rotation, s.end.rotation) = (xf.rotation, xf.rotation);
    }
    if xf.scale != s.shown.scale {
        (s.start.scale, s.end.scale) = (xf.scale, xf.scale);
    }
    s.shown = *xf;
}

/// A tick starts from where the last one ended.
fn tick_start(mut movers: Query<(&mut Transform, &mut Smoothed)>) {
    for (mut xf, mut s) in &mut movers {
        adopt(&mut s, &xf);
        let end = s.end;
        s.start = end;
        s.shown = end;
        xf.set_if_neq(end);
    }
}

fn tick_end(mut movers: Query<(&Transform, &mut Smoothed)>) {
    for (xf, mut s) in &mut movers {
        s.end = *xf;
        s.shown = *xf;
    }
}

/// Between ticks: the blend by the frame's overstep.
fn show(time: Res<Time<Fixed>>, mut movers: Query<(&mut Transform, &mut Smoothed)>) {
    let t = time.overstep_fraction();
    for (mut xf, mut s) in &mut movers {
        adopt(&mut s, &xf);
        let blend = Transform { translation: s.start.translation.lerp(s.end.translation, t), rotation: s.start.rotation.slerp(s.end.rotation, t), scale: s.end.scale };
        s.shown = blend;
        xf.set_if_neq(blend);
    }
}

fn log_drawn(time: Res<Time<Real>>, me: Query<&GlobalTransform, (With<crate::controls::LocalPlayer>, With<crate::player::Player>)>) {
    for g in &me {
        let p = g.translation();
        info!("smooth: {:.5} {:.4} {:.4} {:.4}", time.delta_secs(), p.x, p.y, p.z);
    }
}
