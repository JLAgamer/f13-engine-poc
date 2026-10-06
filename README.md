# f13-rs

A from-scratch Rust reimplementation of *Friday the 13th: The Game* gameplay rules, intended to run on Bevy
using assets and tuning loaded from your own legitimate install.

This repository contains no game files, extracted assets or decompiled code. Gameplay values are not
shipped. They are passed in at runtime, and a loader will read them from the player's install.

## Crates
- `f13-core`: engine-independent rules (no Bevy dependency), unit-tested
  - `killer_abilities`: Jason's ability unlock order and intervals, recharge, availability, rage
  - `fear`: counselor fear driven by fear events (delays, caps, auto-deactivate, throttled updates, cosmetic easing)
  - `stats`: counselor stats and stat-modified values (e.g. Composure scaling fear gain)
  - `counselor`: stance, walk/run/sprint rules, stamina, max speed, wounded state
  - `movement`: Unreal 4.18 walking velocity (acceleration, friction, braking)
  - `curve`: float curves matching Unreal 4.18 `FRichCurve` evaluation
  - `spawn`: match-start spawning of vehicles, repair parts and items (the game mode's rules)
- `f13-data`: loads `tuning.json` into the core types, plus the exporter's level layouts and class descriptions,
  and builds a match's actors (`world`)
- `f13-bevy`: Bevy 0.19 plugin (`F13GameplayPlugin`) with components, `FixedUpdate` systems and messages
- `f13-game`: the game client; for now a viewer for the MVP map built from your own export

## Tuning from your own install
Gameplay values are read from your copy of the game, never shipped here:

```
python tools/extract_tuning.py --game "<path to F13Game>" --uassetgui "<path to UAssetGUI.exe>"
```

This writes `~/.f13-rs/tuning.json` (`%USERPROFILE%\.f13-rs\tuning.json` on Windows), which `f13_data::load(&f13_data::default_path())` reads.
Re-run the extractor when a new build reports a different tuning format.
It needs the main pak extracted to loose files, and it checks the game executable's build before reading
any constant from it.

## Meshes and maps from your own install
`tools/f13-export` is a small C# tool (.NET 10, using [CUE4Parse](https://github.com/FabianFG/CUE4Parse)) that
reads your install's loose files and writes glTF meshes, PNG textures, material info and level layouts (see the note below):

```
dotnet run --project tools/f13-export -- --game "<path to F13Game>" --out "<export dir>" mvp
```

- `levels/<map>.json`: every mesh placement (including foliage instances), landscape and gameplay actor, in Unreal
  space. `f13_data::level` loads it and converts transforms to Bevy space.
- `materials.json`: per material: exported file, effective blend mode and two-sidedness, its master material and
  every parameter it ends up with (instance overrides, then the master's defaults), and for landscape materials the
  layer table. The game draws cabins, terrain and water with shaders ported from the game's own (re-export after
  pulling if your export predates this). `tools/f13-export ... materials` rewrites just this file.
- `*.dds` next to each texture PNG: the game's own block-compressed texture data with its mip chain (BC1/BC3/BC5),
  which the game loads in preference to the PNG (a quarter of the GPU memory, no shimmer at a distance).
  `tools/f13-export ... dds` writes just these.
- `counselors/<Blueprint>.json` and `anims/*.anim`: the counselor's character as the Blueprint assembles it (base
  body, default outfit with its default materials, face on the head bone, hair on the face, skin masks), the shared
  skeleton's bones with their retargeting modes, and her animations sampled per frame. `tools/f13-export ... counselor`
  writes just these.
- `anims.json`: length and root motion of the montages the rules use (dodges, stumbles); dodges and stumbles move
  the counselor by it, as the game's root motion does.
- `classes.json`: Blueprint classes the match can spawn (cabinets, items, cars, doors...): parents, defaults and
  components in actor space. Level files also carry each gameplay actor's properties and components.
- Keep the export directory outside this repository.
- Both tools need the game's content extracted to loose files under `SummerCamp/Content` (the usual modding
  setup). A stock install keeps it in `Content/Paks/*.pak`, which isn't a standard Unreal pak, so it can't be read
  yet; the tools detect this and say so.

View the map (Crystal Lake, small; `--layout` picks one of its four cabin layouts):

```
cargo run -p f13-game --release -- --export "<export dir>" --layout 1
```

By default you play Vanessa, in her default outfit and animated (idle, walk/run/sprint, crouch, combat strafing,
dodges, stumbles, first aid; `--capsule` for the old placeholder): WASD move (running by default), Shift sprint, Caps Lock toggle walk, Ctrl crouch, C combat stance (double-tap
A/D/S to dodge), click to capture the mouse,
Esc to release. The HUD shows health, stamina, fear and speed. `--demo` sprints in a straight line and logs stamina every second.
Counselors stumble as in the game (by fear, stamina and wounds); fear events aren't pushed yet, so `--fear 0-100` sets
a fixed fear level for testing. Health is the game's (wounded at 50, dead at 0); nothing deals damage yet, so
`--health 1-100` starts you hurt.
`--fly` gives a free camera instead. Add `--grass` for grass and flowers.

Items, cabinets and cars spawn as in the game (`--seed n` repeats a match; the log lists where the cars and parts
are). E picks up (preferring what the camera looks at), searches drawers and installs parts, G drops the held
item, 1-3 use the small item in that pocket. Doors: E opens/closes, hold E from inside a closed door to lock or
unlock it, or to put up / take down its barricade (from outside, hold E unlocks a locked one). Windows: E opens a
closed window or climbs through an open or broken one; sprint + E dives through (breaking the glass, 20 damage);
hold E closes an open one (first aid heals after its 2 s animation). Repairs are the game's skill check: hold E
and, when the needle reaches a pip on the dial in the HUD, press its mouse button (R = right, L = left); hits speed
the repair up, misses roll it back. Escape by car: install the gas
can and the battery, get in the driver's seat (E), hold E to start with the keys in your pockets (about 5 s), drive (WASD) into
a car escape zone; F gets out. `--near-car` starts you next to a car. `--bot-escape` plays the whole escape
automatically (teleporting between goals) and exits when done; `--bot-heal` fetches and uses a first aid spray;
`--bot-dodge` dodges in each direction and checks the distance and stamina; `--bot-door` runs 16 checks on a door, a
barricaded door and a window.
For screenshots: `--screenshot out.png` (after loading; `--screenshot-after s` waits longer), `--camera
x,y,z,tx,ty,tz` (eye and look-at, relative to her feet), `--pose <animation>@<seconds>` (holds her in one animation).
`--speed x` runs the game clock faster (bots included; 4 keeps every result the same and cuts a bot run to ~15 s).

## Tests
```
cargo test
```
