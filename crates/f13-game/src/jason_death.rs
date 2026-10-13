//! Killing Jason (MODLOG "Jason's death"): his mask off and Pamela's sweater's stun on him (sweater.rs), a counselor's
//! melee hit puts him in his kill stance with his JasonKillObject (JasonDeath_C, an SCContextKillActor) at him
//! (jason.rs). Its Interactable (ILLInteractableComponent's DistanceLimit 200 cm) takes a counselor with a weapon
//! of its ContextKillComponent's WeaponRequirements (CounselorTwoHandedAxe_C, CounselorMachete_C); its
//! bPlayWeaponSpecificKills picks the takes by her weapon (KillerWeaponKillClasses Counselor_KillJason_Axe_C /
//! _Machete_C: AM_m_cinematic_death_sequence_axe / _machete; VictimWeaponKillClasses Jason_Death_*:
//! AM_cinematic_death_sequence_*), both played from where he kneels (InteractorPositionRequirement
//! LocationAndRotation). Its KillInfo (KillJason_KillInfo_C) scores her KillJason; he's dead and the match ends.
//! The shotgun can't kill him: its Gun takes are in the weapon-specific lists, but WeaponRequirements gates the
//! interaction (FUN_14033a130: her current weapon must be one of its classes) and a gun's hit doesn't start his kill
//! stance. PLACEHOLDERS: its camera splines (Axe_SCSplineCamera, Machete_SCSplineCamera), the prompt.

use bevy::prelude::*;

use crate::player::Player;

pub struct JasonDeathPlugin;

impl Plugin for JasonDeathPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (start_kill, hold_killer).run_if(crate::net::simulating));
    }
}

/// The Interactable's reach (ILLInteractableComponent's DistanceLimit default), m.
const REACH_M: f32 = 2.0;

/// Her take and his, by her weapon's class.
const TAKES: [(&str, &str, &str); 2] = [
    ("CounselorTwoHandedAxe_C", "m_cinematic_death_sequence_axe", "cinematic_death_sequence_axe"),
    ("CounselorMachete_C", "m_cinematic_death_sequence_machete", "cinematic_death_sequence_machete"),
];

/// She's killing him: her take's origin and how long it runs.
#[derive(Component)]
pub struct KillingJason {
    origin: Transform,
    t: f32,
    length: f32,
}

#[allow(clippy::type_complexity)]
fn start_kill(
    mut commands: Commands,
    pads: crate::controls::Pads,
    mut counselors: Query<(Entity, &mut Transform, &mut Player, &crate::interact::Inventory), Without<KillingJason>>,
    items: Query<&crate::props::Item>,
    jasons: Query<(Entity, &crate::jason::Jason, &Transform), Without<Player>>,
    model: Option<Res<crate::character::CounselorCharacter>>,
) {
    let Some(model) = model else { return };
    for (e, mut xf, mut p, inv) in &mut counselors {
        if !p.counselor.alive() || !pads.just_pressed(e, crate::controls::Button::Interact) {
            continue;
        }
        let Some((_, hers, his)) = inv.hand.and_then(|h| items.get(h).ok()).and_then(|i| TAKES.iter().find(|(c, ..)| *c == i.class)) else { continue };
        let Some((je, _, jxf)) = jasons.iter().find(|(_, j, jxf)| j.kneeling() && jxf.translation.distance(xf.translation) <= REACH_M) else { continue };
        info!("jason death: {e} kills Jason ({hers})");
        let (origin, _) = model.place(hers, *jxf);
        *xf = origin;
        p.velocity = Vec2::ZERO;
        let length = model.length(hers).unwrap_or(5.0);
        commands.entity(e).insert((crate::character::PlayAction(hers.to_string()), KillingJason { origin, t: 0.0, length }));
        commands.entity(je).insert(crate::jason::KillJason { take: his, by: e });
    }
}

/// She stays on her take's origin while it plays.
fn hold_killer(mut commands: Commands, time: Res<Time>, mut killers: Query<(Entity, &mut Transform, &mut Player, &mut KillingJason)>) {
    for (e, mut xf, mut p, mut k) in &mut killers {
        k.t += time.delta_secs();
        *xf = k.origin;
        p.velocity = Vec2::ZERO;
        if k.t >= k.length {
            info!("jason death: {e}'s kill is over");
            commands.entity(e).remove::<(KillingJason, crate::character::PlayAction)>();
        }
    }
}

/// Testing (--bot-kill-jason, with --give CounselorMachete_C --jason-health 5): she picks Pamela's sweater up, knocks
/// his mask off, stuns him with the sweater, hits him into his kill stance and kills him.
#[derive(Resource, Default)]
pub struct BotKillJason {
    step: u8,
    t: f32,
    pressed: bool,
}

#[allow(clippy::type_complexity)]
pub(crate) fn bot_kill_jason(
    mut commands: Commands,
    mut bot: ResMut<BotKillJason>,
    time: Res<Time>,
    real: Res<Time<bevy::time::Real>>,
    (mut keys, mut mouse): (ResMut<ButtonInput<KeyCode>>, ResMut<ButtonInput<MouseButton>>),
    items: Query<(&crate::props::Item, &GlobalTransform), Without<crate::interact::Carried>>,
    mut me: Query<(Entity, &mut Transform, Option<&crate::sweater::Sweater>), (With<crate::controls::LocalPlayer>, With<Player>)>,
    jasons: Query<(Entity, &crate::jason::Jason, &Transform), Without<Player>>,
    mut camera: Query<&mut crate::player::PlayerCamera>,
    mut exit: MessageWriter<AppExit>,
) {
    bot.t += time.delta_secs();
    keys.release(KeyCode::KeyE);
    keys.release(KeyCode::KeyQ);
    mouse.release(MouseButton::Left);
    let Ok((me_e, mut xf, sweater)) = me.single_mut() else { return };
    let Some((je, j, jxf)) = jasons.iter().next() else { return };
    // Her machete's hit on him (the swing itself: the fight bot's; items.rs).
    let hit = |commands: &mut Commands, from: Vec3| {
        commands.entity(je).insert(crate::jason::HitJason { by: Some(me_e), stun: None, from, damage: 10.0 });
    };
    let mut face = |xf: &mut Transform, at: Vec3, from: f32| {
        let fwd = jxf.rotation * Vec3::Z;
        xf.translation = Vec3::new(at.x + fwd.x * from, xf.translation.y.max(at.y), at.z + fwd.z * from);
        // Facing him (she faces +Z).
        let to = at - xf.translation;
        xf.rotation = Quat::from_rotation_y(to.x.atan2(to.z));
        if let Ok(mut c) = camera.single_mut() {
            let d = (at + Vec3::Y - (xf.translation + Vec3::Y)).normalize_or_zero();
            c.yaw = (-d.x).atan2(-d.z);
            c.pitch = d.y.asin();
        }
    };
    let fail = |exit: &mut MessageWriter<AppExit>, why: &str| {
        error!("bot: kill jason: {why}");
        exit.write(AppExit::error());
    };
    let once = |bot: &mut BotKillJason| !std::mem::replace(&mut bot.pressed, true);
    let next = |bot: &mut BotKillJason, step: u8| {
        bot.step = step;
        bot.t = 0.0;
        bot.pressed = false;
    };
    match bot.step {
        0 if real.elapsed_secs() > 6.0 && bot.t > 3.0 => {
            let Some((_, g)) = items.iter().find(|(i, _)| crate::sweater::SWEATER_CLASSES.contains(&i.class.as_str())) else { return fail(&mut exit, "no sweater") };
            xf.translation = g.translation() + Vec3::new(0.6, 0.0, 0.0);
            crate::bot::setup(crate::bot::BotSetup::Place(xf.translation.to_array()));
            if let Ok(mut c) = camera.single_mut() {
                let d = (g.translation() - (xf.translation + Vec3::Y)).normalize_or_zero();
                c.yaw = (-d.x).atan2(-d.z);
                c.pitch = d.y.asin();
            }
            next(&mut bot, 1);
        }
        1 => {
            if sweater.is_some() {
                info!("bot: kill jason: wearing the sweater");
                next(&mut bot, 2);
            } else if bot.t > 20.0 {
                fail(&mut exit, "couldn't pick the sweater up");
            } else if (bot.t * 2.0) as u32 % 2 == 0 {
                keys.press(KeyCode::KeyE);
            }
        }
        // His mask: a swing.
        2 => {
            face(&mut xf, jxf.translation, 1.5);
            if !j.mask_on {
                info!("bot: kill jason: his mask is off");
                next(&mut bot, 3);
            } else if once(&mut bot) {
                hit(&mut commands, xf.translation);
            } else if bot.t > 5.0 {
                fail(&mut exit, "his mask stayed on");
            }
        }
        // The sweater.
        3 if bot.t > 3.5 => {
            face(&mut xf, jxf.translation, 3.0);
            if j.sweater_stunned {
                info!("bot: kill jason: he kneels to the sweater");
                next(&mut bot, 4);
            } else if once(&mut bot) {
                keys.press(KeyCode::KeyQ);
            } else if bot.t > 6.0 {
                fail(&mut exit, "the sweater didn't stun him");
            }
        }
        // The hit that puts him in his kill stance.
        4 if bot.t > 0.5 => {
            face(&mut xf, jxf.translation, 1.5);
            if j.kneeling() {
                info!("bot: kill jason: in his kill stance");
                next(&mut bot, 5);
            } else if once(&mut bot) {
                hit(&mut commands, xf.translation);
            } else if bot.t > 5.0 {
                fail(&mut exit, "no kill stance");
            }
        }
        // The kill.
        5 if bot.t > 1.5 => {
            face(&mut xf, jxf.translation, 1.0);
            if j.dead() {
                info!("bot: kill jason: he's dying");
                next(&mut bot, 6);
            } else if bot.t > 9.0 {
                fail(&mut exit, "couldn't kill him");
            } else if (bot.t * 4.0) as u32 % 2 == 0 {
                // (Pressed again: a press the frame after start_kill ran is missed.)
                keys.press(KeyCode::KeyE);
            }
        }
        6 => {
            if matches!(j.state, crate::jason::JasonState::Dead) {
                info!("bot: kill jason: passed");
                exit.write(AppExit::Success);
                next(&mut bot, 9);
            } else if bot.t > 30.0 {
                fail(&mut exit, "he didn't die");
            }
        }
        _ => {}
    }
}
