//! Killing Jason (MODLOG "Jason's death"): his mask off and Pamela's sweater's stun on him (sweater.rs), a counselor's
//! melee hit puts him in his kill stance with his JasonKillObject (JasonDeath_C, an SCContextKillActor) at him
//! (jason.rs). Its Interactable (ILLInteractableComponent's DistanceLimit 200 cm) takes a counselor with a weapon
//! of its ContextKillComponent's WeaponRequirements (CounselorTwoHandedAxe_C, CounselorMachete_C); its
//! bPlayWeaponSpecificKills picks the takes by her weapon (KillerWeaponKillClasses Counselor_KillJason_Axe_C /
//! _Machete_C: AM_m_cinematic_death_sequence_axe / _machete; VictimWeaponKillClasses Jason_Death_*:
//! AM_cinematic_death_sequence_*), both played from where he kneels (InteractorPositionRequirement
//! LocationAndRotation). Its KillInfo (KillJason_KillInfo_C) scores her KillJason; he's dead and the match ends.
//! His mask: knocked off (jason.rs), his KillerMaskPickupClass (J2_MaskPickup_C, an SCKillerMask: an SCItem with no
//! overrides) is spawned and dropped where he stands (TakeDamage 0x14040b600: SpawnActor, then the item's OnDropped
//! vt+0x660 with him and no offset); a counselor picks it up into her pockets (its PickedUpFearEvent
//! PickedUpMaskFearEvent). Carried, she doesn't set off Jason's CounselorInLair (0x1403c41b8; the lair isn't ported).
//! The shotgun can't kill him: its Gun takes are in the weapon-specific lists, but WeaponRequirements gates the
//! interaction (FUN_14033a130: her current weapon must be one of its classes) and a gun's hit doesn't start his kill
//! stance. She's put at its ContextKillComponent (LocationAndRotation; he stays), her take from there; its camera rides
//! her weapon's spline (Axe_SCSplineCamera on AxeSpline, Machete_SCSplineCamera on Machete_Spline), seen by every
//! player (bPlayForEveryone). PLACEHOLDER: the prompt's text (interact.rs Target::KillJason; no widget port).

use bevy::prelude::*;

use crate::player::Player;

pub struct JasonDeathPlugin;

impl Plugin for JasonDeathPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_actor).add_systems(Update, (start_kill, hold_killer, drop_mask).run_if(crate::net::simulating));
    }
}

/// Jason_SackHead_C's KillerMaskPickupClass.
pub const KILLER_MASK_PICKUP: &str = "J2_MaskPickup_C";

/// The match's mask pickup, set aside (props.rs) until his mask is knocked off.
#[derive(Component)]
pub struct KillerMask;

/// His mask off: it drops where he stands, a pickup.
fn drop_mask(mut commands: Commands, jasons: Query<(&crate::jason::Jason, &Transform), Changed<crate::jason::Jason>>, masks: Query<Entity, With<KillerMask>>, mut visibility: Query<&mut Visibility>) {
    for (j, jxf) in &jasons {
        if j.mask_on {
            continue;
        }
        for m in &masks {
            info!("jason death: his mask dropped at {:.1}", jxf.translation);
            commands.entity(m).remove::<KillerMask>();
            crate::interact::drop_at(&mut commands, &mut visibility, m, jxf);
        }
    }
}

/// The Interactable's reach (ILLInteractableComponent's DistanceLimit default), m.
const REACH_M: f32 = 2.0;

/// Her take and his, and the spline its camera rides (KillerWeaponKillClasses' KillCameraNames: Axe_SCSplineCamera on
/// AxeSpline, Machete_SCSplineCamera on Machete_Spline), by her weapon's class.
const TAKES: [(&str, &str, &str, &str); 2] = [
    ("CounselorTwoHandedAxe_C", "m_cinematic_death_sequence_axe", "cinematic_death_sequence_axe", "AxeSpline"),
    ("CounselorMachete_C", "m_cinematic_death_sequence_machete", "cinematic_death_sequence_machete", "Machete_Spline"),
];

/// JasonDeath_C's layout (classes.json, actor space): its ContextKillComponent (where she's put:
/// InteractorPositionRequirement LocationAndRotation; he stays, the victim's default) and its camera splines.
#[derive(Resource, Default)]
pub(crate) struct DeathActor {
    kill: Transform,
    splines: std::collections::HashMap<String, Transform>,
    /// Its Interactable (actor space).
    interact: Vec3,
}

impl DeathActor {
    /// Its Interactable's place with JasonDeath at a Jason standing at `jason` (it's spawned at him).
    pub(crate) fn interact_at(&self, jason: &Transform) -> Vec3 {
        let root = Transform::from_matrix(jason.to_matrix() * Transform::from_rotation(crate::character::FACING).to_matrix());
        root.transform_point(self.interact)
    }
}

/// A weapon class JasonDeath takes (its ContextKillComponent's WeaponRequirements).
pub(crate) fn kills_jason(class: &str) -> bool {
    TAKES.iter().any(|(c, ..)| *c == class)
}

fn load_actor(mut commands: Commands, export: Res<crate::ExportRoot>) {
    let Ok(classes) = f13_data::classes::ClassDb::load(&export.0.join("classes.json")) else { return };
    let Some(info) = classes.get("JasonDeath_C") else {
        warn!("jason death: no JasonDeath_C in classes.json (re-export: f13-export levels): its takes play at him");
        return;
    };
    let local = |c: &f13_data::level::ComponentInfo| crate::to_transform(&c.local.unwrap_or(f13_data::level::Xform::IDENTITY));
    let kill = info.components.iter().find(|c| c.name == "ContextKillComponent").map_or(Transform::IDENTITY, local);
    let splines = info.components.iter().filter(|c| c.kind == "SCCameraSplineComponent").map(|c| (c.name.clone(), local(c))).collect();
    let interact = info.components.iter().find(|c| c.name == "Interactable").map_or(Vec3::ZERO, |c| local(c).translation);
    commands.insert_resource(DeathActor { kill, splines, interact });
}

/// She's killing him: her take's origin, where it ends, and how long it runs.
#[derive(Component)]
pub struct KillingJason {
    origin: Transform,
    end: Transform,
    t: f32,
    length: f32,
}

#[allow(clippy::type_complexity)]
pub(crate) fn start_kill(
    mut commands: Commands,
    pads: crate::controls::Pads,
    mut counselors: Query<(Entity, &mut Transform, &mut Player, &crate::interact::Inventory), Without<KillingJason>>,
    items: Query<&crate::props::Item>,
    jasons: Query<(Entity, &crate::jason::Jason, &Transform), Without<Player>>,
    model: Option<Res<crate::character::CounselorCharacter>>,
    actor: Option<Res<DeathActor>>,
) {
    let Some(model) = model else { return };
    for (e, mut xf, mut p, inv) in &mut counselors {
        if !p.counselor.alive() || !pads.just_pressed(e, crate::controls::Button::Interact) {
            continue;
        }
        let Some(&(_, hers, his, spline)) = inv.hand.and_then(|h| items.get(h).ok()).and_then(|i| TAKES.iter().find(|(c, ..)| *c == i.class)) else { continue };
        let Some((je, _, jxf)) = jasons.iter().find(|(_, j, jxf)| j.kneeling() && jxf.translation.distance(xf.translation) <= REACH_M) else { continue };
        info!("jason death: {e} kills Jason ({hers})");
        // JasonDeath is spawned at him (EnterKillStance's SpawnActor): his actor's transform is its root. She's put at
        // its ContextKillComponent (turned as characters face), and her take starts there.
        let root = Transform::from_matrix(jxf.to_matrix() * Transform::from_rotation(crate::character::FACING).to_matrix());
        let actor = actor.as_deref();
        let at = actor.map_or(*jxf, |a| crate::interact::standing_on(&xf, root.mul_transform(a.kill)));
        let (origin, end) = model.place(hers, at);
        *xf = origin;
        p.velocity = Vec2::ZERO;
        let length = model.length(hers).unwrap_or(5.0);
        commands.entity(e).insert((crate::character::PlayAction(hers.to_string()), KillingJason { origin, end, t: 0.0, length }));
        commands.entity(je).insert(crate::jason::KillJason { take: his, by: e });
        // Its camera (bPlayForEveryone: every player watches): her weapon's spline, in its own frame under the root.
        if let Some(s) = actor.and_then(|a| a.splines.get(spline)) {
            commands.entity(je).insert(crate::kill_camera::KillCam::at(&format!("JasonDeath_{spline}"), root.mul_transform(*s)).for_everyone());
        }
    }
}

/// She stays on her take's origin while it plays (its root motion is in the pose), then stands where it ends.
fn hold_killer(mut commands: Commands, time: Res<Time>, mut killers: Query<(Entity, &mut Transform, &mut Player, &mut KillingJason)>) {
    for (e, mut xf, mut p, mut k) in &mut killers {
        k.t += time.delta_secs();
        *xf = k.origin;
        p.velocity = Vec2::ZERO;
        if k.t >= k.length {
            info!("jason death: {e}'s kill is over");
            *xf = k.end;
            commands.entity(e).remove::<(KillingJason, crate::character::PlayAction)>();
        }
    }
}

/// Where she stands to swing at him (her machete's reach: from 1.5 m its KillVolume passes 0.3 m short of his bodies).
const SWING_M: f32 = 0.9;

/// Testing (--bot-kill-jason, with --give CounselorMachete_C --jason-health 5): she picks Pamela's sweater up, knocks
/// his mask off, stuns him with the sweater, hits him into his kill stance and kills him (run before start_kill, which
/// reads her E as just pressed).
#[derive(Resource, Default)]
pub struct BotKillJason {
    step: u8,
    t: f32,
    pressed: bool,
}

#[allow(clippy::type_complexity)]
pub(crate) fn bot_kill_jason(
    mut bot: ResMut<BotKillJason>,
    time: Res<Time>,
    real: Res<Time<bevy::time::Real>>,
    (mut keys, mut mouse): (ResMut<ButtonInput<KeyCode>>, ResMut<ButtonInput<MouseButton>>),
    items: Query<(&crate::props::Item, &GlobalTransform), Without<crate::interact::Carried>>,
    mut me: Query<(&mut Transform, Option<&crate::sweater::Sweater>), (With<crate::controls::LocalPlayer>, With<Player>)>,
    jasons: Query<(&crate::jason::Jason, &Transform), Without<Player>>,
    mut camera: Query<&mut crate::player::PlayerCamera>,
    mut exit: MessageWriter<AppExit>,
) {
    bot.t += time.delta_secs();
    keys.release(KeyCode::KeyE);
    keys.release(KeyCode::KeyQ);
    mouse.release(MouseButton::Left);
    let Ok((mut xf, sweater)) = me.single_mut() else { return };
    let Some((j, jxf)) = jasons.iter().next() else { return };
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
            face(&mut xf, jxf.translation, SWING_M);
            if !j.mask_on {
                info!("bot: kill jason: his mask is off");
                next(&mut bot, 3);
            } else if (bot.t * 2.0) as u32 % 2 == 0 {
                mouse.press(MouseButton::Left);
            } else if bot.t > 8.0 {
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
            face(&mut xf, jxf.translation, SWING_M);
            if j.kneeling() {
                info!("bot: kill jason: in his kill stance");
                next(&mut bot, 5);
            } else if (bot.t * 2.0) as u32 % 2 == 0 {
                mouse.press(MouseButton::Left);
            } else if bot.t > 8.0 {
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

/// Testing (--bot-mask, with --give CounselorMachete_C --jason-health 5 --jason-delay 600): she knocks his mask off and
/// picks it up into her pockets.
#[derive(Resource, Default)]
pub struct BotMask {
    step: u8,
    t: f32,
}

#[allow(clippy::type_complexity)]
pub(crate) fn bot_mask(
    mut bot: ResMut<BotMask>,
    time: Res<Time>,
    real: Res<Time<bevy::time::Real>>,
    (mut keys, mut mouse): (ResMut<ButtonInput<KeyCode>>, ResMut<ButtonInput<MouseButton>>),
    items: Query<(Entity, &crate::props::Item, &GlobalTransform), (Without<crate::interact::Carried>, Without<KillerMask>)>,
    mut me: Query<(&mut Transform, &crate::interact::Inventory), (With<crate::controls::LocalPlayer>, With<Player>)>,
    jasons: Query<(&crate::jason::Jason, &Transform), Without<Player>>,
    mut camera: Query<&mut crate::player::PlayerCamera>,
    mut exit: MessageWriter<AppExit>,
) {
    bot.t += time.delta_secs();
    keys.release(KeyCode::KeyE);
    mouse.release(MouseButton::Left);
    let Ok((mut xf, inv)) = me.single_mut() else { return };
    let Some((j, jxf)) = jasons.iter().next() else { return };
    let mut look = |xf: &mut Transform, at: Vec3| {
        let to = at - xf.translation;
        xf.rotation = Quat::from_rotation_y(to.x.atan2(to.z));
        if let Ok(mut c) = camera.single_mut() {
            let d = (at - (xf.translation + Vec3::Y)).normalize_or_zero();
            c.yaw = (-d.x).atan2(-d.z);
            c.pitch = d.y.asin();
        }
    };
    let mut fail = |why: &str| {
        error!("bot: mask: {why}");
        exit.write(AppExit::error());
    };
    match bot.step {
        0 if real.elapsed_secs() > 6.0 && bot.t > 3.0 => {
            let fwd = jxf.rotation * Vec3::Z;
            xf.translation = Vec3::new(jxf.translation.x + fwd.x * SWING_M, xf.translation.y.max(jxf.translation.y), jxf.translation.z + fwd.z * SWING_M);
            crate::bot::setup(crate::bot::BotSetup::Place(xf.translation.to_array()));
            look(&mut xf, jxf.translation);
            bot.step = 1;
            bot.t = 0.0;
        }
        1 => {
            look(&mut xf, jxf.translation);
            if !j.mask_on {
                info!("bot: mask: knocked off");
                bot.step = 2;
                bot.t = 0.0;
            } else if bot.t > 8.0 {
                fail("his mask stayed on");
                bot.step = 9;
            } else if (bot.t * 2.0) as u32 % 2 == 0 {
                mouse.press(MouseButton::Left);
            }
        }
        // At the mask, once it's down.
        2 if bot.t > 1.0 => {
            let Some((_, _, g)) = items.iter().find(|(_, i, _)| i.class == KILLER_MASK_PICKUP) else {
                fail("no mask on the ground");
                bot.step = 9;
                return;
            };
            let at = g.translation();
            info!("bot: mask: on the ground at {at:.1}");
            xf.translation = at + Vec3::new(0.6, 0.0, 0.0);
            crate::bot::setup(crate::bot::BotSetup::Place(xf.translation.to_array()));
            look(&mut xf, at);
            bot.step = 3;
            bot.t = 0.0;
        }
        3 => {
            // (Pocketed, it's Carried: out of `items`.)
            let picked = items.iter().all(|(_, i, _)| i.class != KILLER_MASK_PICKUP);
            if picked && !inv.pockets.is_empty() {
                info!("bot: mask: in her pockets");
                info!("bot: mask: passed");
                exit.write(AppExit::Success);
                bot.step = 9;
            } else if bot.t > 15.0 {
                fail("couldn't pick it up");
                bot.step = 9;
            } else if (bot.t * 2.0) as u32 % 2 == 0 {
                keys.press(KeyCode::KeyE);
            }
        }
        _ => {}
    }
}
