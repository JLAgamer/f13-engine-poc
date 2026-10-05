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
- `materials.json`: material name, exported file, effective blend mode and two-sidedness.
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

By default you play Vanessa (a placeholder capsule; `--model` shows her mesh, which has no outfit or animation
yet): WASD move (running by default), Shift sprint, Caps Lock toggle walk, Ctrl crouch, click to capture the mouse,
Esc to release. The HUD shows stamina and speed. `--demo` sprints in a straight line and logs stamina every second.
`--fly` gives a free camera instead. Add `--grass` for grass and flowers.

Items, cabinets and cars spawn as in the game (`--seed n` repeats a match; the log lists where the cars and parts
are). E picks up, searches drawers and installs parts (hold), G drops the held item. Escape by car: install the gas
can and the battery, get in the driver's seat (E), hold E to start with the keys in your pockets, drive (WASD) into
a car escape zone; F gets out. `--near-car` starts you next to a car. `--bot-escape` plays the whole escape
automatically (teleporting between goals) and exits when done.

## Tests
```
cargo test
```
