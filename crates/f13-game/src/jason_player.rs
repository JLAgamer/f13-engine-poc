//! Playing as Jason (`--play-jason`): the keyboard and mouse drive Jason and the camera follows him, while a bot plays
//! the counselor (the escape bot unless another `--bot-*` is given; it doesn't close the game when it's done).
//!
//! The bot plays her through the engine's input state, so Jason's input is taken from the raw keyboard and mouse
//! events before the engine sees them: the player's keys never reach her.
//!
//! Keys: WASD walk, Shift run, mouse look; left click swing, middle click heavy swing, right click grab, F throw a
//! knife, T set a trap, left click again while holding her to kill her, 1 Shift, 2 Morph (to the morph point nearest 30 m ahead of where he looks), 3 Stalk, 4 Sense; while holding her 1-5 pick a kill. Doors in
//! his way open or break as he walks into them; E destroys a fuse box he's at (the power in its grid goes out). Esc frees the mouse, a click takes it again.

use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{AccumulatedMouseMotion, MouseButtonInput};
use bevy::input::{ButtonInput, ButtonState};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};

/// How far ahead of where he looks Morph aims (the game picks the spot on the map).
pub const MORPH_AHEAD_M: f32 = 30.0;
const CAMERA_DISTANCE_M: f32 = 3.5;
const CAMERA_HEIGHT_M: f32 = 2.3;

/// The player is Jason.
#[derive(Resource)]
pub struct PlayJason;

pub struct PlayJasonPlugin {
    /// The player is Jason from the start (`--play-jason`); else only once the host makes them Jason ([`become_jason`]).
    pub active: bool,
    /// Scripted input (testing): look at her, walk up, a knife from range, a swing, then grabs.
    pub demo: bool,
}

/// Makes the player Jason mid-match (the host's `jason`): his controls take over the keyboard and mouse and the
/// camera, looking where he faces; the counselor is left without a player.
pub fn become_jason(commands: &mut Commands, jason_facing: Quat) {
    let f = jason_facing * Vec3::Z;
    commands.insert_resource(JasonControls { yaw: (-f.x).atan2(-f.z), pitch: -0.15, ..default() });
    commands.insert_resource(PlayJason);
}

impl Plugin for PlayJasonPlugin {
    fn build(&self, app: &mut App) {
        let on = resource_exists::<PlayJason>;
        if self.demo {
            app.add_systems(PreUpdate, demo.after(capture).before(bevy::input::InputSystems).run_if(on));
        }
        if self.active {
            app.insert_resource(PlayJason).init_resource::<JasonControls>();
        }
        app.add_systems(PreUpdate, capture.before(bevy::input::InputSystems).run_if(on))
            .add_systems(Update, (look, hud).run_if(on))
            .add_systems(
                Update,
                view.after(crate::player::follow_camera).after(crate::hiding::hiding_camera).before(crate::kill_camera::kill_camera).run_if(on),
            )
            .add_systems(Startup, spawn_hud);
    }
}

/// The player's Jason input: held keys and buttons (from the raw events), the view's yaw and pitch, and the presses
/// not yet used by a fixed step.
#[derive(Resource, Default)]
pub struct JasonControls {
    keys: ButtonInput<KeyCode>,
    mouse: ButtonInput<MouseButton>,
    pub yaw: f32,
    pub pitch: f32,
    latched: Presses,
    /// 4: Sense, asked for and not yet used (senses.rs takes it).
    pub sense: bool,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Presses {
    pub attack: bool,
    pub heavy: bool,
    pub grab: bool,
    pub knife: bool,
    pub trap: bool,
    pub shift: bool,
    pub morph: bool,
    pub stalk: bool,
    /// E: destroy the fuse box at hand; holding her, use the placed kill at hand.
    pub interact: bool,
    /// 1-5 while he holds her: that grab kill (jason.rs KILLS).
    pub kill: Option<usize>,
}

/// One fixed step's input.
#[derive(Clone, Copy, Debug)]
pub struct JasonInput {
    /// Where to walk (world XZ, unit or zero).
    pub movement: Vec2,
    pub run: bool,
    /// Where he looks: flat (world XZ, unit) and in 3D.
    pub forward: Vec2,
    pub look: Vec3,
    pub presses: Presses,
}

impl JasonControls {
    /// This step's input; the presses are used up.
    pub fn take(&mut self) -> JasonInput {
        let (sin, cos) = self.yaw.sin_cos();
        let forward = Vec2::new(-sin, -cos);
        let right = Vec2::new(cos, -sin);
        let mut m = Vec2::ZERO;
        for (k, v) in [(KeyCode::KeyW, forward), (KeyCode::KeyS, -forward), (KeyCode::KeyD, right), (KeyCode::KeyA, -right)] {
            if self.keys.pressed(k) {
                m += v;
            }
        }
        let look = Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0) * Vec3::NEG_Z;
        JasonInput { movement: m.normalize_or_zero(), run: self.keys.pressed(KeyCode::ShiftLeft), forward, look, presses: std::mem::take(&mut self.latched) }
    }
}

/// Takes the raw keyboard and mouse button events (before the engine's input state sees them).
fn capture(
    mut controls: ResMut<JasonControls>,
    mut keyboard: ResMut<Messages<KeyboardInput>>,
    mut buttons: ResMut<Messages<MouseButtonInput>>,
    mut cursor: Single<&mut CursorOptions>,
) {
    let c = &mut *controls;
    c.keys.clear();
    c.mouse.clear();
    for e in keyboard.drain() {
        match e.state {
            ButtonState::Pressed => c.keys.press(e.key_code),
            ButtonState::Released => c.keys.release(e.key_code),
        }
    }
    for e in buttons.drain() {
        match e.state {
            ButtonState::Pressed => c.mouse.press(e.button),
            ButtonState::Released => c.mouse.release(e.button),
        }
    }
    if c.keys.just_pressed(KeyCode::Escape) {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
    let locked = cursor.grab_mode == CursorGrabMode::Locked;
    if c.mouse.just_pressed(MouseButton::Left) && !locked {
        // The first click takes the mouse (no swing).
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
        return;
    }
    let p = &mut c.latched;
    p.attack |= c.mouse.just_pressed(MouseButton::Left);
    p.heavy |= c.mouse.just_pressed(MouseButton::Middle);
    p.grab |= c.mouse.just_pressed(MouseButton::Right);
    p.knife |= c.keys.just_pressed(KeyCode::KeyF);
    p.trap |= c.keys.just_pressed(KeyCode::KeyT);
    p.shift |= c.keys.just_pressed(KeyCode::Digit1);
    p.morph |= c.keys.just_pressed(KeyCode::Digit2);
    p.stalk |= c.keys.just_pressed(KeyCode::Digit3);
    let sense = c.keys.just_pressed(KeyCode::Digit4);
    let digits = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5];
    if let Some(i) = digits.iter().position(|&k| c.keys.just_pressed(k)) {
        p.kill = Some(i);
    }
    c.sense |= sense;
    p.interact |= c.keys.just_pressed(KeyCode::KeyE);
}

/// The scripted player: what a player would press, by distance to her.
fn demo(
    mut controls: ResMut<JasonControls>,
    jason: Query<(&crate::jason::Jason, &Transform)>,
    her: Query<&Transform, (With<crate::player::Player>, Without<crate::jason::Jason>)>,
    fuse_boxes: Query<&crate::power::FuseBox>,
    mut done: Local<(bool, bool, f32)>,
    time: Res<Time>,
) {
    let (Some((j, jxf)), Some(h)) = (jason.iter().next(), her.iter().next()) else { return };
    let d = h.translation - jxf.translation;
    let dist = Vec2::new(d.x, d.z).length();
    // Yaw so that the view's forward (-sin, -cos) points at her.
    controls.yaw = (-d.x).atan2(-d.z);
    controls.pitch = -0.15;
    // Up to her, not into her.
    if dist > 1.3 {
        controls.keys.press(KeyCode::KeyW);
    } else {
        controls.keys.release(KeyCode::KeyW);
    }
    controls.keys.press(KeyCode::ShiftLeft);
    // Sense whenever it's ready while she's not close.
    controls.sense |= dist > 20.0;
    let now = time.elapsed_secs();
    let (knife, swung, last) = &mut *done;
    debug!("jason demo: {:?} at {:.1}, {dist:.1} m from her, velocity {:.0}", j.state, jxf.translation, j.velocity);
    // Holding her: a kill picked by its key (3: the third).
    if matches!(j.state, crate::jason::JasonState::Holding { t } if t > 1.5) {
        controls.latched.kill = Some(2);
    }
    if !matches!(j.state, crate::jason::JasonState::Hunting) || now - *last < 1.0 {
        return;
    }
    // An intact fuse box at hand: break it (E).
    if fuse_boxes.iter().any(|f| !f.destroyed && Vec2::new(f.interact.x - jxf.translation.x, f.interact.z - jxf.translation.z).length() * 100.0 <= f.reach) {
        *last = now;
        controls.latched.interact = true;
        info!("jason demo: breaking the fuse box at hand");
    } else if !*knife && j.knives > 0 && (8.0..20.0).contains(&dist) {
        *knife = true;
        *last = now;
        controls.latched.knife = true;
        info!("jason demo: knife at {dist:.1} m");
    } else if dist < 1.9 && !*swung {
        *swung = true;
        *last = now;
        controls.latched.attack = true;
        info!("jason demo: swing at {dist:.1} m");
    } else if dist < 2.3 && *swung {
        *last = now;
        controls.latched.grab = true;
        info!("jason demo: grab at {dist:.1} m");
    }
}

/// Mouse look.
fn look(mut controls: ResMut<JasonControls>, motion: Res<AccumulatedMouseMotion>, cursor: Single<&CursorOptions>) {
    if cursor.grab_mode == CursorGrabMode::Locked {
        controls.yaw -= motion.delta.x * 0.003;
        controls.pitch = (controls.pitch - motion.delta.y * 0.003).clamp(-1.2, 0.6);
    }
}

/// The camera behind Jason, where he looks (the counselor's camera, moved: its settings and listener stay).
fn view(
    controls: Res<JasonControls>,
    jason: Query<&Transform, (With<crate::jason::Jason>, Without<crate::player::PlayerCamera>)>,
    mut camera: Single<&mut Transform, With<crate::player::PlayerCamera>>,
) {
    let Some(j) = jason.iter().next() else { return };
    let rot = Quat::from_euler(EulerRot::YXZ, controls.yaw, controls.pitch, 0.0);
    let target = j.translation + Vec3::Y * CAMERA_HEIGHT_M;
    camera.translation = target - rot * Vec3::NEG_Z * CAMERA_DISTANCE_M;
    camera.rotation = rot;
}

#[derive(Component)]
struct JasonHud;

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont { font_size: FontSize::Px(18.0), ..default() },
        Node { position_type: PositionType::Absolute, bottom: Val::Px(16.0), left: Val::Px(16.0), ..default() },
        JasonHud,
    ));
}

/// What Jason has: his state, knives, traps, the abilities ready.
fn hud(jason: Query<(&crate::jason::Jason, Option<&crate::senses::Knowledge>)>, mut text: Single<&mut Text, With<JasonHud>>) {
    let Some((j, know)) = jason.iter().next() else { return };
    use f13_core::killer_abilities::{Ability, Blockers};
    let ready: Vec<&str> = [(Ability::Shift, "1 Shift"), (Ability::Morph, "2 Morph"), (Ability::Stalk, "3 Stalk"), (Ability::Sense, "4 Sense")]
        .iter()
        .filter(|(a, _)| know.is_some_and(|k| k.abilities.is_available(*a, &Blockers::default())))
        .map(|(_, n)| *n)
        .collect();
    let s = format!(
        "Jason: {:?}\nknives {} [F]  traps {} [T]  ready: {}\nLMB swing  MMB heavy  RMB grab  Shift run",
        j.state,
        j.knives,
        j.traps,
        if ready.is_empty() { "-".to_string() } else { ready.join(", ") }
    );
    if text.0 != s {
        text.0 = s;
    }
}
