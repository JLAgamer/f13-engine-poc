//! Tommy Jarvis, the game mode's hunter (MODLOG "Tommy Jarvis: the call and the spawn"):
//! - the CB radio (HunterCBRadio_C, SCCBRadio; one in each match's large items): an 11 s hold with its minigame
//!   (interact.rs), then SCCBRadio::OnInteract: the hunter's timer, her CallHunter score, the radio done;
//! - StartHunterTimer: AttemptSpawnHunter after GameState's HunterResponseTime (DefaultGame.ini: 10 s). It spawns him
//!   when nobody is pending, he isn't in yet, the counselors aren't just one, and more than one of them is out (dead
//!   or escaped) or all are: in a random one of the players dead (not by suicide), else the players escaped, else the
//!   AI counselors out (FUN_140377d90); none, the timer again;
//! - SpawnHunter: at a random SCHunterPlayerStart at least MinHunterSpawnDistFromJason (1000 cm, in 2D) from Jason, her
//!   player now the hunter's class (Hunter_Counselor_C: Tommy) with the HunterWeaponClass (Shotgun_C).
//!
//! Here the chosen counselor is revived as him (her entity: her player's link stays) at the start, his looks and stats
//! hers (character.rs restyle), the match's spare shotgun (props.rs: one Shotgun_C set aside) in his hands.
//! - a player's hunter first has his start's intro (FUN_14040fe60: its SpawnIntroSequence, Hunter_CLSmall, played for his
//!   player: cinematic.rs HUNTER), and HunterIntroComplete comes on a timer of its length less 1 s (at least 1 s;
//!   FUN_14042d340: the sequence player's GetLength); an AI counselor comes at once. The match waits meanwhile.
//! PLACEHOLDERS: the starts' "free" check (all are), HunterIntroComplete's pawn setup beyond the class and weapon, his
//! player state's bIsHunter in the tallies.

use bevy::prelude::*;

use crate::player::Player;

pub struct TommyPlugin;

impl Plugin for TommyPlugin {
    fn build(&self, app: &mut App) {
        crate::match_flow::register_badges(app);
        app.init_resource::<HunterTimer>()
            .add_systems(Startup, (hunter_starts, intro_length))
            .add_observer(play_intro)
            .add_systems(Update, hunter_timer.run_if(crate::net::simulating))
            .add_systems(Update, leave_spectating);
    }
}

/// The CB radio: its interaction's spot (world) and reach, its hold (HoldTimeLimit), and whether Tommy has been called.
#[derive(Component)]
pub struct CbRadio {
    pub class: String,
    pub interact: Vec3,
    pub reach: f32,
    pub hold_time: f32,
    pub called: bool,
}

/// An SCHunterPlayerStart.
#[derive(Component)]
pub struct HunterStart;

/// The shotgun set aside for him (the HunterWeaponClass).
#[derive(Component)]
pub struct HunterWeapon;

/// HunterIntroComplete's kit beside his weapon (SCGameMode's WalkieTalkieClass, MapClass, MedSprayClass,
/// PocketKnifeClass: constructor literals WalkieTalkie_C, Map_C, BP_FirstAid_C, BP_PocketKnife_C), set aside like it.
pub const HUNTER_KIT: [&str; 4] = ["WalkieTalkie_C", "Map_C", "BP_FirstAid_C", "BP_PocketKnife_C"];

/// One of his kit's items, set aside (props.rs).
#[derive(Component)]
pub struct HunterKit;

/// SCGameMode's TommyJarvisBadge (0x888): the first of its constructor's four badge literals (SecondWind, then
/// Reinforcements, Ninja, ChadFace for EscapeWithPolice, Survive and SurviveWithFear, the fields' order; PLACEHOLDER: the
/// pairing is by that order).
const TOMMY_BADGE: &str = "SecondWind";

/// He's Tommy (bIsHunter).
#[derive(Component)]
pub struct Hunter;

/// The hunter's timer (seconds left; HunterTimeRemaining shows it), the one chosen while his intro plays (HunterToSpawn,
/// his start, the time left to HunterIntroComplete) and whether he's in (bIsHunterSpawned).
#[derive(Resource, Default)]
pub struct HunterTimer {
    pub remaining: Option<f32>,
    pub intro: Option<(Entity, Transform, f32)>,
    pub spawned: bool,
}

impl HunterTimer {
    /// He's on his way (the Hunt mode doesn't end the match while he is).
    pub fn pending(&self) -> bool {
        !self.spawned && (self.remaining.is_some() || self.intro.is_some())
    }
}

/// The counselor who comes as Tommy, while his intro plays (net.rs sends it to his client).
#[derive(Component)]
pub struct HunterIntro;

/// The intro's length (its sequence's playback range), when exported.
#[derive(Resource)]
struct IntroLength(f32);

fn intro_length(mut commands: Commands, export: Res<crate::ExportRoot>) {
    let path = export.0.join("sequences").join(format!("{}.json", crate::cinematic::HUNTER.master));
    let v: Option<serde_json::Value> = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok());
    match v.and_then(|v| Some(v["End"].as_f64()? - v["Start"].as_f64()?)) {
        Some(len) => commands.insert_resource(IntroLength(len as f32)),
        None => warn!("tommy: no {} (f13-export sequences): he comes at once", path.display()),
    }
}

/// His intro, on the machine he's played on (FUN_14040fe60 is his player controller's).
fn play_intro(add: On<Add, HunterIntro>, mut commands: Commands, local: Query<(), With<crate::controls::LocalPlayer>>, enabled: Option<Res<crate::cinematic::IntroEnabled>>, export: Res<crate::ExportRoot>) {
    if !local.contains(add.entity) {
        return;
    }
    if enabled.is_some() && crate::cinematic::available(&export.0, &crate::cinematic::HUNTER) {
        info!("tommy: his intro");
        commands.insert_resource(crate::cinematic::PlayCinematic(crate::cinematic::HUNTER));
    }
}

/// The CB radio's take (interact.rs repair_take: its montage's sections).
pub const RADIO_TAKE: &str = "use_cb_radio";
/// SCGameState HunterResponseTime (DefaultGame.ini), s.
const RESPONSE_TIME_S: f32 = 10.0;
/// SCGameMode_Hunt MinHunterSpawnDistFromJason (DefaultGame.ini), cm.
const MIN_FROM_JASON_CM: f32 = 1000.0;
/// SCGameMode's HunterCharacterClass (Hunter_Counselor_C), as a counselor class.
pub const HUNTER_CLASS: &str = "Hunter";

/// The map's SCHunterPlayerStarts (level actors).
fn hunter_starts(mut commands: Commands, level: Res<crate::Level>) {
    let mut n = 0;
    for a in level.layouts.iter().flat_map(|l| l.actors.iter()).filter(|a| a.class == "SCHunterPlayerStart") {
        let Some(t) = &a.transform else { continue };
        commands.spawn((crate::to_transform(t), HunterStart, Name::new(a.name.clone())));
        n += 1;
    }
    info!("tommy: {n} hunter starts");
}

/// SCCBRadio::OnInteract: the call went through.
pub fn call_hunter(commands: &mut Commands, radio: &mut CbRadio, timer: &mut HunterTimer, player: Entity) {
    radio.called = true;
    timer.remaining = Some(RESPONSE_TIME_S);
    info!("tommy: called on the CB radio: he comes in {RESPONSE_TIME_S} s if he can");
    commands.trigger(crate::match_flow::ClassScore::new(player, &radio.class, "CallHunterScoreEvent"));
}

#[allow(clippy::type_complexity)]
fn hunter_timer(
    mut commands: Commands,
    time: Res<Time>,
    mut timer: ResMut<HunterTimer>,
    mut counselors: Query<(Entity, &mut Player, &mut Transform, Has<crate::vehicle::Escaped>, Has<crate::controls::LocalPlayer>, Has<crate::net::PlayedBy>, Option<&mut crate::interact::Inventory>), Without<crate::jason::Jason>>,
    starts: Query<&Transform, (With<HunterStart>, Without<Player>)>,
    jason: Query<&Transform, (With<crate::jason::Jason>, Without<Player>)>,
    weapon: Query<Entity, With<HunterWeapon>>,
    tuning: Res<crate::player::PlayerTuning>,
    mut rng: ResMut<crate::GameRng>,
    intro: Option<Res<IntroLength>>,
    kit: Query<(Entity, &crate::props::Item), With<HunterKit>>,
) {
    // His intro playing: HunterIntroComplete as its timer runs out.
    if let Some((chosen, at, left)) = timer.intro {
        let left = left - time.delta_secs();
        if left > 0.0 {
            timer.intro = Some((chosen, at, left));
            return;
        }
        timer.intro = None;
        commands.entity(chosen).remove::<HunterIntro>();
        timer.spawned = true;
        revive(&mut commands, chosen, at, &mut counselors, &weapon, &kit, &tuning);
        return;
    }
    let Some(left) = timer.remaining.filter(|_| !timer.spawned) else { return };
    let left = left - time.delta_secs();
    if left > 0.0 {
        timer.remaining = Some(left);
        return;
    }
    // AttemptSpawnHunter.
    let n = counselors.iter().count();
    let out = counselors.iter().filter(|(_, p, _, escaped, ..)| !p.counselor.alive() || *escaped).count();
    let can = n != 1 && (out > 1 || out == n);
    let player = |local: bool, played: bool| local || played;
    let pick = |rng: &mut crate::GameRng, list: &[Entity]| -> Option<Entity> { (!list.is_empty()).then(|| list[rng.0.index(list.len())]) };
    let dead: Vec<Entity> = counselors.iter().filter(|(_, p, _, escaped, local, played, _)| player(*local, *played) && !p.counselor.alive() && !escaped).map(|c| c.0).collect();
    let escaped: Vec<Entity> = counselors.iter().filter(|(_, _, _, escaped, local, played, _)| player(*local, *played) && *escaped).map(|c| c.0).collect();
    let ai: Vec<Entity> = counselors.iter().filter(|(_, p, _, escaped, local, played, _)| !player(*local, *played) && (!p.counselor.alive() || *escaped)).map(|c| c.0).collect();
    let chosen = if can { pick(&mut rng, &dead).or_else(|| pick(&mut rng, &escaped)).or_else(|| pick(&mut rng, &ai)) } else { None };
    let Some(chosen) = chosen else {
        info!("tommy: nobody to come as him ({out} of {n} counselors out): the timer again");
        timer.remaining = Some(RESPONSE_TIME_S);
        return;
    };
    // SpawnHunter: a start far enough from Jason (PLACEHOLDER: none is: the farthest).
    let jat = jason.iter().next().map(|t| t.translation);
    let flat = |a: Vec3, b: Vec3| Vec2::new(a.x - b.x, a.z - b.z).length() * 100.0;
    let all: Vec<Transform> = starts.iter().copied().collect();
    let far: Vec<Transform> = all.iter().copied().filter(|s| jat.is_none_or(|j| flat(s.translation, j) >= MIN_FROM_JASON_CM)).collect();
    let at = if far.is_empty() {
        all.iter().copied().max_by(|a, b| jat.map_or(0.0, |j| flat(a.translation, j)).total_cmp(&jat.map_or(0.0, |j| flat(b.translation, j))))
    } else {
        Some(far[rng.0.index(far.len())])
    };
    let Some(at) = at else {
        warn!("tommy: no hunter start in this map");
        timer.remaining = None;
        return;
    };
    timer.remaining = None;
    // A player's hunter: his intro first (SpawnHunter: a PlayerController HunterToSpawn plays his start's intro, the
    // timer at its length less 1 s, at least 1 s); else HunterIntroComplete at once.
    let played = counselors.get(chosen).is_ok_and(|(_, _, _, _, local, played, _)| local || played);
    if let (true, Some(len)) = (played, intro.as_deref()) {
        let wait = (len.0 - 1.0).max(1.0);
        info!("tommy: {chosen}'s intro at {:.1}: he comes in {wait:.1} s", at.translation);
        timer.intro = Some((chosen, at, wait));
        commands.entity(chosen).insert(HunterIntro);
        return;
    }
    timer.spawned = true;
    revive(&mut commands, chosen, at, &mut counselors, &weapon, &kit, &tuning);
}

/// HunterIntroComplete: the chosen counselor revived as him at his start.
#[allow(clippy::type_complexity)]
fn revive(
    commands: &mut Commands,
    chosen: Entity,
    at: Transform,
    counselors: &mut Query<(Entity, &mut Player, &mut Transform, Has<crate::vehicle::Escaped>, Has<crate::controls::LocalPlayer>, Has<crate::net::PlayedBy>, Option<&mut crate::interact::Inventory>), Without<crate::jason::Jason>>,
    weapon: &Query<Entity, With<HunterWeapon>>,
    kit: &Query<(Entity, &crate::props::Item), With<HunterKit>>,
    tuning: &crate::player::PlayerTuning,
) {
    let Ok((_, mut p, mut xf, ..)) = counselors.get_mut(chosen) else { return };
    // Revived as him: full health, his stats, at the start facing its way (Unreal +X; she faces +Z).
    let mut c = f13_core::counselor::Counselor::new(tuning.stats_of(HUNTER_CLASS));
    c.health = tuning.counselor.health.max_health;
    p.counselor = c;
    p.velocity = Vec2::ZERO;
    p.fear = 0.0;
    p.montage = None;
    p.root_motion = None;
    p.hidden = false;
    let (yaw, _, _) = at.rotation.to_euler(EulerRot::YXZ);
    xf.translation = at.translation;
    xf.rotation = Quat::from_rotation_y(yaw + std::f32::consts::FRAC_PI_2);
    info!("tommy: {chosen} comes as Tommy Jarvis at {:.1}", at.translation);
    commands
        .entity(chosen)
        .remove::<(crate::vehicle::Escaped, crate::voice::Spectating, crate::character::PlayAction, crate::jason::Held, crate::items::Trapped, crate::blood::Gore)>()
        .insert((crate::character::CounselorClass(HUNTER_CLASS.to_string()), Hunter));
    // His shotgun.
    if let (Ok(w), Ok((.., Some(mut inv)))) = (weapon.single(), counselors.get_mut(chosen)) {
        inv.hand = Some(w);
        commands.entity(w).remove::<HunterWeapon>();
        info!("tommy: his shotgun in his hands");
    }
    // The rest of his kit (FUN_1402e2a10 each onto his new pawn): the walkie-talkie and map his special items, the
    // first aid spray and pocket knife his pockets.
    if let Ok((.., Some(mut inv))) = counselors.get_mut(chosen) {
        for (e, item) in kit {
            if item.special {
                inv.special.push(e);
            } else {
                inv.pockets.push(e);
            }
            commands.entity(e).remove::<HunterKit>();
        }
        info!("tommy: his kit: {} special, {} in his pockets", inv.special.len(), inv.pockets.len());
    }
    // His player state's TommyJarvisBadge.
    commands.trigger(crate::match_flow::EarnBadge { who: chosen, badge: TOMMY_BADGE });
}

/// Tommy's player looks on no more: back in his own view.
fn leave_spectating(mine: Query<(), (With<Hunter>, With<crate::controls::LocalPlayer>, Added<Hunter>)>, spectate: Option<ResMut<crate::spectate::Spectate>>) {
    if mine.is_empty() {
        return;
    }
    if let Some(mut s) = spectate {
        s.mode = crate::spectate::Mode::None;
        s.player = None;
    }
}

/// Testing (--bot-tommy): she calls on the CB radio, then another counselor and she die; she comes back as Tommy.
#[derive(Resource, Default)]
pub struct BotTommy {
    step: u8,
    t: f32,
    tries: u32,
}

#[allow(clippy::type_complexity)]
pub(crate) fn bot_tommy(
    mut commands: Commands,
    mut bot: ResMut<BotTommy>,
    time: Res<Time>,
    real: Res<Time<bevy::time::Real>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    (mut mouse, tuning): (ResMut<ButtonInput<MouseButton>>, Res<crate::player::PlayerTuning>),
    timer: Res<HunterTimer>,
    radios: Query<(&CbRadio, &GlobalTransform)>,
    mut me: Query<(Entity, &mut Transform, &mut Player, Option<&crate::character::CounselorClass>), With<crate::controls::LocalPlayer>>,
    mut others: Query<&mut Player, Without<crate::controls::LocalPlayer>>,
    mut camera: Query<&mut crate::player::PlayerCamera>,
    interaction: Query<&crate::interact::Interaction, With<crate::controls::LocalPlayer>>,
    mut exit: MessageWriter<AppExit>,
) {
    let on_radio = interaction.single().is_ok_and(|i| i.on_cb_radio());
    bot.t += time.delta_secs();
    // Each frame's presses are its own (a key held across frames reads as one press).
    mouse.release(MouseButton::Left);
    mouse.release(MouseButton::Right);
    let Ok((e, mut xf, mut p, class)) = me.single_mut() else { return };
    // (A client's bot: the call and her class are what reach it.)
    let hunter = class.is_some_and(|c| c.0 == HUNTER_CLASS);
    let called = timer.remaining.is_some() || radios.iter().any(|(r, _)| r.called);
    match bot.step {
        0 if real.elapsed_secs() > 6.0 && bot.t > 3.0 => {
            let Some((r, g)) = radios.iter().next() else {
                error!("bot: tommy: no CB radio in this match");
                exit.write(AppExit::error());
                bot.step = 9;
                return;
            };
            // In front of its Interactable (each side in turn until it's what she'd use), facing it.
            let at = r.interact;
            let side = [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z][(bot.tries % 4) as usize];
            bot.tries += 1;
            let out = g.rotation() * side;
            let stand = Vec3::new(at.x + out.x * 0.7, xf.translation.y.max(at.y - 1.0), at.z + out.z * 0.7);
            xf.translation = stand;
            crate::bot::setup(crate::bot::BotSetup::Place(stand.to_array()));
            if let Ok(mut c) = camera.single_mut() {
                let d = (at - (stand + Vec3::Y)).normalize_or_zero();
                c.yaw = (-d.x).atan2(-d.z);
                c.pitch = d.y.asin();
            }
            info!("bot: tommy: at the CB radio ({:.1})", at);
            bot.step = 1;
            bot.t = 0.0;
        }
        1 if !on_radio && bot.t > 1.0 => {
            info!("bot: tommy: would use {} here: another side", interaction.single().map_or(String::new(), |i| i.target_name()));
            bot.step = 0;
            if bot.tries > 8 {
                error!("bot: tommy: the CB radio isn't usable from any side");
                exit.write(AppExit::error());
                bot.step = 9;
            }
        }
        1 if on_radio || called => {
            keys.press(KeyCode::KeyE);
            // Its minigame's pips: each one's key when the needle is in its middle.
            if (bot.t * 2.0) as u32 != ((bot.t - time.delta_secs()) * 2.0) as u32 {
                let i = interaction.single().ok();
                info!("bot: tommy: holding: target {}, progress {:?}", i.map_or(String::new(), |i| i.target_name()), i.and_then(|i| i.hold.as_ref()).map(|h| (h.started, h.progress(&tuning.minigame))));
            }
            if let Some(h) = interaction.single().ok().and_then(|i| i.hold.as_ref()).filter(|h| h.started) {
                if let Some(pip) = h.current.map(|c| h.pips[c]) {
                    if h.progress(&tuning.minigame) >= (pip.position + pip.width / 2.0) / 360.0 {
                        mouse.press(if pip.key == 0 { MouseButton::Right } else { MouseButton::Left });
                    }
                }
            }
            if called {
                keys.release(KeyCode::KeyE);
                info!("bot: tommy: called after {:.1}s", bot.t);
                bot.step = 2;
                bot.t = 0.0;
            } else if bot.t > 90.0 {
                error!("bot: tommy: the call didn't go through");
                exit.write(AppExit::error());
                bot.step = 9;
            }
        }
        2 if bot.t > 1.0 => {
            // Two out (another counselor and her): the hunter can come.
            if let Some(mut o) = others.iter_mut().find(|o| o.counselor.alive()) {
                o.counselor.health = 0.0;
            }
            p.counselor.health = 0.0;
            crate::bot::setup(crate::bot::BotSetup::KillAnother);
            crate::bot::setup(crate::bot::BotSetup::Die);
            info!("bot: tommy: two counselors dead");
            bot.step = 3;
            bot.t = 0.0;
        }
        3 => {
            if hunter && p.counselor.alive() {
                info!("bot: tommy: she came back as Tommy Jarvis {:.1}s after dying (entity {e})", bot.t);
                bot.step = 4;
                bot.t = 0.0;
            } else if bot.t > 40.0 {
                error!("bot: tommy: she never came back as Tommy");
                exit.write(AppExit::error());
                bot.step = 9;
            }
        }
        4 if bot.t > 3.0 && bot.tries < 100 => {
            let path = std::env::temp_dir().join("f13-bot-tommy.png");
            info!("bot: screenshot {}", path.display());
            commands.spawn(bevy::render::view::screenshot::Screenshot::primary_window()).observe(bevy::render::view::screenshot::save_to_disk(path));
            bot.tries = 100;
        }
        4 if bot.t > 5.0 => {
            info!("bot: tommy: passed");
            exit.write(AppExit::Success);
            bot.step = 9;
        }
        _ => {}
    }
}
