//! A counselor's controls: the buttons she's holding and where her controller looks, as a networked player's input
//! would carry them (Unreal's PlayerController sends its input and its ControlRotation to the server). Every counselor
//! has her own; the one this machine plays ([`LocalPlayer`]) takes the keyboard's and mouse's each frame, a remote
//! player's come over the network and a bot's from its script. The counselor's systems read these, not the keyboard.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// The counselor this machine plays: its camera, HUD, listener and keyboard.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct LocalPlayer;

/// A counselor's buttons (the game's DefaultInput.ini bindings as f13-rs maps them).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum Button {
    /// W, S, D, A.
    Forward,
    Back,
    Right,
    Left,
    /// Shift.
    Sprint,
    /// Ctrl.
    Crouch,
    /// X: the run toggle (CSLR_Run).
    Walk,
    /// C: combat stance.
    Combat,
    /// E: interact (press and hold).
    Interact,
    /// G: drop the held item.
    Drop,
    /// Z: use the held item.
    UseItem,
    /// 1, 2, 3: the pocket slots.
    Slot1,
    Slot2,
    Slot3,
    /// M: the map.
    Map,
    /// F: the flashlight (and the car's lights).
    Flashlight,
    /// H: the car's horn.
    Horn,
    /// Space: hold breath, the car's handbrake.
    Space,
    /// Left mouse: attack, fire.
    Primary,
    /// Right mouse: block, aim.
    Secondary,
    /// Q: her ability (CSLR_UseAbility: Pamela's sweater; on release).
    Ability,
}

impl Button {
    const fn bit(self) -> u32 {
        1 << self as u8
    }
}

/// The keyboard key or mouse button this machine reads for each [`Button`].
const BINDINGS: [(Button, Bind); 21] = [
    (Button::Forward, Bind::Key(KeyCode::KeyW)),
    (Button::Back, Bind::Key(KeyCode::KeyS)),
    (Button::Right, Bind::Key(KeyCode::KeyD)),
    (Button::Left, Bind::Key(KeyCode::KeyA)),
    (Button::Sprint, Bind::Key(KeyCode::ShiftLeft)),
    (Button::Crouch, Bind::Key(KeyCode::ControlLeft)),
    (Button::Walk, Bind::Key(KeyCode::KeyX)),
    (Button::Combat, Bind::Key(KeyCode::KeyC)),
    (Button::Interact, Bind::Key(KeyCode::KeyE)),
    (Button::Drop, Bind::Key(KeyCode::KeyG)),
    (Button::UseItem, Bind::Key(KeyCode::KeyZ)),
    (Button::Slot1, Bind::Key(KeyCode::Digit1)),
    (Button::Slot2, Bind::Key(KeyCode::Digit2)),
    (Button::Slot3, Bind::Key(KeyCode::Digit3)),
    (Button::Map, Bind::Key(KeyCode::KeyM)),
    (Button::Flashlight, Bind::Key(KeyCode::KeyF)),
    (Button::Horn, Bind::Key(KeyCode::KeyH)),
    (Button::Space, Bind::Key(KeyCode::Space)),
    (Button::Primary, Bind::Mouse(MouseButton::Left)),
    (Button::Secondary, Bind::Mouse(MouseButton::Right)),
    (Button::Ability, Bind::Key(KeyCode::KeyQ)),
];

#[derive(Clone, Copy)]
enum Bind {
    Key(KeyCode),
    Mouse(MouseButton),
}

/// A counselor's controls this frame: [`ButtonInput`]'s pressed / just pressed / just released per [`Button`], and
/// her control rotation (yaw about up, pitch; radians, Bevy's camera convention).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Controls {
    pressed: u32,
    just_pressed: u32,
    just_released: u32,
    pub yaw: f32,
    pub pitch: f32,
}

impl Controls {
    pub fn pressed(&self, b: Button) -> bool {
        self.pressed & b.bit() != 0
    }

    pub fn just_pressed(&self, b: Button) -> bool {
        self.just_pressed & b.bit() != 0
    }

    pub fn just_released(&self, b: Button) -> bool {
        self.just_released & b.bit() != 0
    }

    /// Her stick: x right, y forward (from W, A, S, D).
    pub fn movement(&self) -> Vec2 {
        let mut m = Vec2::ZERO;
        for (b, v) in [(Button::Forward, Vec2::Y), (Button::Back, -Vec2::Y), (Button::Right, Vec2::X), (Button::Left, -Vec2::X)] {
            if self.pressed(b) {
                m += v;
            }
        }
        m
    }

    /// The next frame's controls from these buttons held (a remote or scripted player's): what changed since this
    /// frame is just pressed or just released.
    pub fn next(&self, held: u32, yaw: f32, pitch: f32) -> Controls {
        Controls { pressed: held, just_pressed: held & !self.pressed, just_released: self.pressed & !held, yaw, pitch }
    }

    /// [`Controls::next`] with the buttons also pressed or released in between (`presses`, `releases`: a tap shorter
    /// than the gap between two updates still counts).
    pub fn next_with(&self, held: u32, presses: u32, releases: u32, yaw: f32, pitch: f32) -> Controls {
        let n = self.next(held, yaw, pitch);
        Controls { just_pressed: n.just_pressed | presses, just_released: n.just_released | releases, ..n }
    }

    /// The same buttons held, none pressed or released this frame.
    pub fn held_still(&self) -> Controls {
        Controls { just_pressed: 0, just_released: 0, ..*self }
    }

    /// The buttons held, as [`Controls::next`] takes them.
    pub fn held(&self) -> u32 {
        self.pressed
    }

    /// The buttons pressed and released this frame (bits as [`Controls::held`]).
    pub fn edges(&self) -> (u32, u32) {
        (self.just_pressed, self.just_released)
    }
}

/// A counselor's buttons and look, wherever they come from: the local counselor's are this machine's keyboard, mouse
/// and camera, read live (as the bots press them); another's are her [`Controls`].
#[derive(bevy::ecs::system::SystemParam)]
pub struct Pads<'w, 's> {
    keys: Res<'w, ButtonInput<KeyCode>>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    camera: Query<'w, 's, &'static crate::player::PlayerCamera>,
    local: Query<'w, 's, (), With<LocalPlayer>>,
    remote: Query<'w, 's, &'static Controls>,
}

impl Pads<'_, '_> {
    fn read(&self, who: Entity, b: Button, key: impl Fn(&ButtonInput<KeyCode>, KeyCode) -> bool, button: impl Fn(&ButtonInput<MouseButton>, MouseButton) -> bool, bits: impl Fn(&Controls) -> u32) -> bool {
        if self.local.contains(who) {
            match BINDINGS.iter().find(|(x, _)| *x == b).map(|(_, bind)| *bind) {
                Some(Bind::Key(k)) => key(&self.keys, k),
                Some(Bind::Mouse(m)) => button(&self.mouse, m),
                None => false,
            }
        } else {
            self.remote.get(who).is_ok_and(|c| bits(c) & b.bit() != 0)
        }
    }

    pub fn pressed(&self, who: Entity, b: Button) -> bool {
        self.read(who, b, |k, c| k.pressed(c), |m, c| m.pressed(c), |c| c.pressed)
    }

    pub fn just_pressed(&self, who: Entity, b: Button) -> bool {
        self.read(who, b, |k, c| k.just_pressed(c), |m, c| m.just_pressed(c), |c| c.just_pressed)
    }

    pub fn just_released(&self, who: Entity, b: Button) -> bool {
        self.read(who, b, |k, c| k.just_released(c), |m, c| m.just_released(c), |c| c.just_released)
    }

    /// Her stick: x right, y forward.
    pub fn movement(&self, who: Entity) -> Vec2 {
        let mut m = Vec2::ZERO;
        for (b, v) in [(Button::Forward, Vec2::Y), (Button::Back, -Vec2::Y), (Button::Right, Vec2::X), (Button::Left, -Vec2::X)] {
            if self.pressed(who, b) {
                m += v;
            }
        }
        m
    }

    /// Her control rotation (yaw, pitch).
    pub fn look(&self, who: Entity) -> (f32, f32) {
        if self.local.contains(who) {
            if let Ok(c) = self.camera.single() {
                return (c.yaw, c.pitch);
            }
        }
        self.remote.get(who).map_or((0.0, 0.0), |c| (c.yaw, c.pitch))
    }
}

pub struct ControlsPlugin;

impl Plugin for ControlsPlugin {
    fn build(&self, app: &mut App) {
        // The local counselor's, kept for sending (the network layer reads them).
        app.add_systems(PreUpdate, local_controls.after(bevy::input::InputSystems));
    }
}

/// The local counselor's controls from the keyboard and mouse, and her camera's look.
fn local_controls(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    camera: Query<&crate::player::PlayerCamera>,
    mut local: Query<&mut Controls, With<LocalPlayer>>,
) {
    let Ok(mut c) = local.single_mut() else { return };
    let (mut pressed, mut just_pressed, mut just_released) = (0, 0, 0);
    for (b, bind) in BINDINGS {
        let (p, jp, jr) = match bind {
            Bind::Key(k) => (keys.pressed(k), keys.just_pressed(k), keys.just_released(k)),
            Bind::Mouse(m) => (mouse.pressed(m), mouse.just_pressed(m), mouse.just_released(m)),
        };
        pressed |= if p { b.bit() } else { 0 };
        just_pressed |= if jp { b.bit() } else { 0 };
        just_released |= if jr { b.bit() } else { 0 };
    }
    let (yaw, pitch) = camera.single().map_or((c.yaw, c.pitch), |cam| (cam.yaw, cam.pitch));
    *c = Controls { pressed, just_pressed, just_released, yaw, pitch };
}
