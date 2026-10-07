# f13-rs

A from-scratch Rust reimplementation of *Friday the 13th: The Game* gameplay rules, intended to run on Bevy
using assets and tuning loaded from your own legitimate install.

This repository contains no game files, extracted assets or decompiled code. Gameplay values are not
shipped. They are passed in at runtime, and a loader will read them from the player's install.

## Crates
- `f13-core`: engine-independent rules (no Bevy dependency), unit-tested
  - `killer_abilities`: Jason's ability unlock order and intervals, recharge, availability, rage
  - `killer_moves`: Shift (wind-up, speed, length), Stalk (timer, slower standing still), Morph's destination, the
    teleport fade
  - `fear`: counselor fear driven by fear events (delays, caps, auto-deactivate, throttled updates, cosmetic easing)
  - `stats`: counselor stats and stat-modified values (e.g. Composure scaling fear gain)
  - `counselor`: stance, walk/run/sprint rules, stamina, max speed, wounded state
  - `movement`: Unreal 4.18 walking velocity (acceleration, friction, braking)
  - `curve`: float curves matching Unreal 4.18 `FRichCurve` evaluation
  - `spawn`: match-start spawning of vehicles, repair parts and items (the game mode's rules)
  - `game_mode`: the match's states and one-second timer, when it ends, and the XP its score events award
  - `collision`: the capsule mover over the game's collision (walls, steps, slopes, falling)
  - `nav`: paths for computer-controlled characters over the same collision (grid A*)
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
- `killers/Jason_SackHead.json` and `anims/jason/*.anim`: Jason Part 2 the same way (body, sack mask, his pickaxe in
  its hand socket), his gameplay values (speeds, attack and grab ranges, attack delay, weapon damage: Blueprint
  overrides, else the C++ constructor's) and his animations with their notify windows (when a swing can hit).
  `tools/f13-export ... killer` writes just these.
- `collision.json`: each static mesh's simple collision (convex point sets); level placements and class components
  say whether they block counselors (their collision profile, read from the game's DefaultEngine.ini), and
  BlockingVolumes carry their hulls, and each mesh its physical surface (footsteps); each landscape's
  `surfaces.json` gives its paint layers' surfaces. `tools/f13-export ... collision` writes just these (and the level
  files).
- `sounds/`: the SoundCues f13-game plays (footsteps, and every cue the exported classes name) as node graphs (random, modulator, mixer, looping, attenuation...) with their
  waves decoded (Ogg Vorbis), and `footsteps.json`, the counselor's footstep sounds by surface, decoded from her
  animation Blueprint's script. `tools/f13-export ... sounds` writes just these.
- `anims.json`: length and root motion of the montages the rules use (dodges, stumbles); dodges and stumbles move
  the counselor by it, as the game's root motion does.
- `classes.json`: Blueprint classes the match can spawn (cabinets, items, cars, doors...): parents, defaults and
  components in actor space. Level files also carry each gameplay actor's properties and components.
- Keep the export directory outside this repository.
- Both tools need the game's content extracted to loose files under `SummerCamp/Content` (the usual modding
  setup). A stock install keeps it in `Content/Paks/*.pak`, which isn't a standard Unreal pak, so it can't be read
  yet; the tools detect this and say so.

Play on the map (Crystal Lake, small). Each match gets one of its four cabin layouts, picked at random as the game
picks it (`--seed n` repeats a match; `--layout 1-4` picks the layout yourself):

```
cargo run -p f13-game --release -- --export "<export dir>"
```

By default you play Vanessa, in her default outfit and animated (idle, walk/run/sprint, crouch, combat strafing,
dodges, stumbles, first aid; `--capsule` for the old placeholder): WASD move (running by default), Shift sprint, Caps Lock toggle walk, Ctrl crouch, C combat stance (double-tap
A/D/S to dodge), click to capture the mouse,
Esc to release. The HUD shows health, stamina, fear and speed. `--demo` sprints in a straight line and logs stamina every second.
Counselors stumble as in the game (by fear, stamina and wounds). Fear comes from the game's fear events: darkness
(out of reach of the map's lamps and cabin lights), being outdoors and alone, wounded, Jason near (by distance), Jason
in view, grabbed, hiding (calms her) and holding a weapon; it raises stumbles and lets Jason's Sense reach further.
`--fear 0-100` holds it at a fixed level instead. Health is the game's (wounded at 50, dead at 0); `--health 1-100`
starts you hurt.

The night is lit as the game lights it, from the export:
- the moon, its height fog and the sky colour come from the clear-weather level (CLsm_RainOff);
- the map's 162 lamps, porch and cabin lights;
- the fixed exposure and ambient light of the map's post-process volume.

F turns your flashlight on and off (the game's settings: cone, range, colour, the way it turns with you). While it's
on, darkness doesn't scare you. `--flashlight` starts with it on.

Sound: your footsteps are the game's, by what you step on (dirt, grass, leaves, gravel, rock, wooden floors...) and
whether you walk or run, panned and fading with distance as the game sets them; doors (opening, closing, locked,
Jason's blows, breaking), windows and barricades play their own cues. Music, Jason, items and the car aren't heard
yet.

Not yet:
- Enlighten's bounce light;
- shadows from lamps;
- colour grading;
- the sky's clouds and stars;
- rain;
- power cuts.

Jason (Part 2) starts at the layout's killer start and hunts you: he walks or runs (his own speeds), opens doors in
his way (his own door-opening animations) and breaks down locked or barricaded ones with the game's break-down
move (an axe chop every 2 s: three on a cabin door, one on Higgins Haven's outer doors, twice that through a
barricade; the door splinters as it weakens), or once enraged bursts straight through any closed door, swings his pickaxe (20 damage, one swing
per 1.3 s, hitting during the swing's attack window) and grabs you; a grab ends in the pickaxe's Throat Hack kill.
He's a computer player that only knows what he perceives: what he sees (100 m, the counselor's visibility distance),
the sound pings your noise puts on his screen (the game's formula: sprinting is heard far off, walking at average
Stealth isn't, standing still is silent) and Sense (unlocked 46 s in; it reaches further with your fear and late in
the match, and finds hiders only once he's enraged). Out of sight, he goes where he last saw, heard or sensed you and
searches the hiding spots around there. His other abilities work as the game's:
- Morph (ready 1 s in) puts him on the map's morph point nearest a far-off clue, a second after he vanishes.
- Stalk (256 s in) lasts 20 s, ten times longer standing still. It has no effect yet (it silences his music).
- Shift (136 s in) winds up for 0.7 s, then he rushes at you, unseen, at 10 m/s for up to 4 s, and grabs you
  out of it.

When he uses them is the computer player's choice. He finds his way on a navmesh built at load time from the map's collision
with the game's own settings, like the game's runtime navmesh (about 4 s in the background; the HUD shows its
progress, and `--nav-debug` draws it around you).
`--no-jason` leaves him out, `--jason-delay s` gives you a head start, and `--bot-jason` waits to be killed
(screenshots of the swing, grab and kill go to the temp folder). `--bot-abilities` gives him every ability at the
start and tells him where you are: he Morphs over, Stalks, Shifts at you and kills you. The counselor bots below leave
him out.
Hiding: E at a wardrobe, outhouse, bed or tent hides you (the game's enter, idle and exit animations; the view is
the spot's hiding camera, turned with the mouse), E again leaves. If Jason watched you get in, or lost you nearby, he
searches it: an empty spot breaks, an occupied one ends in its hiding spot kill. `--bot-hide` hides out of his sight
and comes out after 60 s; `--bot-hide-seen` hides where he sees and passes when he finds her. `--bot-locked-in` and
`--bot-barred-in` shut her in behind a locked or barricaded door and pass when he breaks it down in a hit per 20 of
its health (10 barricaded) and kills her; `--bot-raged-in` does it with him enraged (one burst).
`--fly` gives a free camera instead. Add `--grass` for grass and flowers.

The match runs as the game's offline mode does. The 20-minute timer (RoundTime) starts once you're in the world
and shows at the top of the screen. The match ends when you die or escape (after its first 10 s), or when the time
runs out. The end-of-match outro waits 10 s, then the results list the XP you scored and their total for 20 s,
then the game closes. The game would go back to the lobby here.
XP comes from the game's score events, where the game awards them: picking up weapons and car parts, repairs,
first aid, barricading, firecrackers, traps, hitting and stunning Jason, breaking his grab, escaping, and the match's
own (time bonus, unscathed, match complete, team escaped). Each one shows on the right as it's scored.
`--round-time s` shortens the match for testing. The level intro and outro sequences aren't played.

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
automatically (teleporting between goals), waits for the match's results and exits; `--bot-heal` fetches and uses a first aid spray;
`--bot-dodge` dodges in each direction and checks the distance and stamina; `--bot-door` runs 16 checks on a door, a
barricaded door and a window.
Against Jason: left mouse swings the weapon in your hands (W + left mouse: the heavy swing; each weapon's stun chance
and durability are the game's), fires the flare gun or shotgun at what the middle of the screen shows (one shot), or
places a bear trap in front of you; 1-3 throw firecrackers from that pocket (they go off 1.5 s later and stun him if
he comes within 2 m). In combat stance, hold right mouse to block with a weapon: his blow does a fraction of its
damage (the game's BlockingDamageMod: a fifth, less with more Strength and Luck) and wears the weapon; a grab goes
through a block (`--bot-block` checks it). Step into an armed bear trap (yours too) and it bites for 50 damage and holds you: mash A / D to open it, as the
game's stun minigame runs (`--bot-trap`). A pocket knife frees you the moment he grabs you; otherwise mash A / D to struggle free. M opens
the map (when you carry one). The walkie-talkie only matters with other players. A dead counselor goes limp (a
ragdoll from her physics asset; in a kill, where the kill animation hands her over, as in the game). `--give Class,Class` starts with items (e.g. `--give BaseballBat_C,BP_PocketKnife_C`);
`--bot-fight` uses them on Jason and passes once he's been stunned and recovers.
`--perf` logs where a frame's time goes every 5 s (built with `--features profile`, also the costliest systems): the main world's schedules, the render world's system sets, GPU
pass times and entity counts (vsync off, to measure the real cost).
For screenshots: `--screenshot out.png` (after loading; `--screenshot-after s` waits longer), `--camera
x,y,z,tx,ty,tz` (eye and look-at, relative to her feet), `--pose <animation>@<seconds>` (holds her in one animation).
`--speed x` runs the game clock faster (bots included; 4 keeps every result the same and cuts a bot run to ~15 s).

## Tests
```
cargo test
```
