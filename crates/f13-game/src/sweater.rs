//! Pamela's sweater (SCPamelaSweater, PamelasSweater_C; spawned in Jason's shack: Pamela_SweaterSpawner,
//! bSpawnOnInitialize; MODLOG "Jason's death"):
//! - only a female counselor can pick it up (its CanInteractWith 0x140374e00: an SCCounselorCharacter whose vt+0xc70,
//!   GetIsFemale by its use, PLACEHOLDER identification); the pickup equips it (FUN_1403cc260: bHasPamelasSweater, its
//!   ability hers, SERVER_SetWearingSweater);
//! - its ability (SweaterAbilityClass: Blueprints/CounselorAbilities/Active/PamelasSweater, type 5, AbilityRange 500,
//!   bInstant) on CSLR_UseAbility's release (Q), once: CanUseAbility (0x1403a79a0) wants her not stumbling, stunned,
//!   grabbed, in her stance 3 or 5, in a special move or a context kill, and a killer within AbilityRange (3D) not in
//!   a context kill or morphing; Activate's area (0x1403a4a40) then calls the killer's FUN_14040b3c0 for each in range
//!   (2D): his PamelasSweaterStun (10 s, its "Resist" minigame) and the flag at 0x1291 (the kill's condition).
//! PLACEHOLDERS: her sweater outfit (SweaterOutfit, OnRep_WearingSweater) isn't shown; her SweaterStunnedJasonBadge.

use bevy::prelude::*;

use crate::player::Player;

pub struct SweaterPlugin;

impl Plugin for SweaterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (equip, use_sweater).chain().run_if(crate::net::simulating));
    }
}

/// The sweater's classes (SCPamelaSweater's Blueprints).
pub const SWEATER_CLASSES: [&str; 2] = ["PamelasSweater_C", "PamelasSweater_Small_C"];
/// Its ability's AbilityRange (cm).
const ABILITY_RANGE_CM: f32 = 500.0;

/// She has Pamela's sweater on (bHasPamelasSweater); `used`: its ability is spent.
#[derive(Component, Debug)]
pub struct Sweater {
    pub used: bool,
}

/// The pickup equips it.
fn equip(mut commands: Commands, players: Query<(Entity, &crate::interact::Inventory), (With<Player>, Without<Sweater>)>, items: Query<&crate::props::Item>) {
    for (e, inv) in &players {
        if inv.special.iter().any(|&i| items.get(i).is_ok_and(|it| SWEATER_CLASSES.contains(&it.class.as_str()))) {
            info!("sweater: {e} put Pamela's sweater on");
            commands.entity(e).insert(Sweater { used: false });
        }
    }
}

#[allow(clippy::type_complexity)]
fn use_sweater(
    mut commands: Commands,
    pads: crate::controls::Pads,
    mut wearers: Query<(Entity, &mut Sweater, &Player, &Transform, (Has<crate::jason::Held>, Has<crate::jason::Staggered>, Has<crate::interact::MoveToSpot>))>,
    jasons: Query<(Entity, &crate::jason::Jason, &Transform), Without<Player>>,
) {
    for (e, mut s, p, xf, (held, staggered, moving)) in &mut wearers {
        if s.used || !pads.just_released(e, crate::controls::Button::Ability) {
            continue;
        }
        if !p.counselor.alive() || held || staggered || moving {
            info!("sweater: can't use it now (alive {}, held {held}, staggered {staggered}, moving {moving})", p.counselor.alive());
            continue;
        }
        let near = jasons.iter().find(|(_, j, jxf)| !j.in_context_kill() && !j.morphing() && jxf.translation.distance(xf.translation) * 100.0 < ABILITY_RANGE_CM);
        let Some((je, ..)) = near else {
            info!("sweater: Jason isn't near enough");
            continue;
        };
        s.used = true;
        info!("sweater: {e} used Pamela's sweater on Jason");
        commands.entity(je).insert(crate::jason::HitJason { by: Some(e), stun: Some(crate::jason::Stun::Sweater), from: xf.translation, damage: 0.0 });
    }
}

/// Testing (--bot-sweater): she picks Pamela's sweater up, stands by Jason and uses it; he kneels.
#[derive(Resource, Default)]
pub struct BotSweater {
    step: u8,
    t: f32,
    pressed: bool,
}

#[allow(clippy::type_complexity)]
pub(crate) fn bot_sweater(
    mut bot: ResMut<BotSweater>,
    time: Res<Time>,
    real: Res<Time<bevy::time::Real>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    items: Query<(&crate::props::Item, &GlobalTransform), Without<crate::interact::Carried>>,
    mut me: Query<(&mut Transform, Option<&Sweater>), (With<crate::controls::LocalPlayer>, With<Player>)>,
    jasons: Query<(&crate::jason::Jason, &Transform), Without<Player>>,
    mut camera: Query<&mut crate::player::PlayerCamera>,
    mut exit: MessageWriter<AppExit>,
) {
    bot.t += time.delta_secs();
    keys.release(KeyCode::KeyE);
    keys.release(KeyCode::KeyQ);
    let Ok((mut xf, sweater)) = me.single_mut() else { return };
    let mut look = |xf: &Transform, at: Vec3| {
        if let Ok(mut c) = camera.single_mut() {
            let d = (at - (xf.translation + Vec3::Y)).normalize_or_zero();
            c.yaw = (-d.x).atan2(-d.z);
            c.pitch = d.y.asin();
        }
    };
    match bot.step {
        0 if real.elapsed_secs() > 6.0 && bot.t > 3.0 => {
            let Some((_, g)) = items.iter().find(|(i, _)| SWEATER_CLASSES.contains(&i.class.as_str())) else {
                error!("bot: sweater: none in this match");
                exit.write(AppExit::error());
                bot.step = 9;
                return;
            };
            let at = g.translation();
            xf.translation = at + Vec3::new(0.6, 0.0, 0.0);
            crate::bot::setup(crate::bot::BotSetup::Place(xf.translation.to_array()));
            look(&xf, at);
            info!("bot: sweater: at Pamela's sweater ({at:.1})");
            bot.step = 1;
            bot.t = 0.0;
        }
        1 => {
            if sweater.is_some() {
                info!("bot: sweater: wearing it");
                bot.step = 2;
                bot.t = 0.0;
            } else if bot.t > 20.0 {
                error!("bot: sweater: couldn't pick it up");
                exit.write(AppExit::error());
                bot.step = 9;
            } else if (bot.t * 2.0) as u32 % 2 == 0 {
                keys.press(KeyCode::KeyE);
            }
        }
        2 if bot.t > 1.0 => {
            let Some((_, jxf)) = jasons.iter().next() else { return };
            let fwd = jxf.rotation * Vec3::Z;
            xf.translation = jxf.translation + fwd * 4.0;
            crate::bot::setup(crate::bot::BotSetup::Place(xf.translation.to_array()));
            look(&xf, jxf.translation + Vec3::Y);
            bot.step = 3;
            bot.t = 0.0;
        }
        3 if bot.t > 0.1 => {
            // CSLR_UseAbility is bound on release: a press (one frame), then its release.
            if !bot.pressed {
                keys.press(KeyCode::KeyQ);
                bot.pressed = true;
            } else if jasons.iter().any(|(j, _)| j.sweater_stunned) {
                info!("bot: sweater: Jason kneels to it");
                bot.step = 4;
                bot.t = 0.0;
            } else if bot.t > 6.0 {
                error!("bot: sweater: Jason didn't kneel");
                exit.write(AppExit::error());
                bot.step = 9;
            }
        }
        4 if bot.t > 2.0 => {
            info!("bot: sweater: passed");
            exit.write(AppExit::Success);
            bot.step = 9;
        }
        _ => {}
    }
}
