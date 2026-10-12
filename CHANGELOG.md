# Changelog

All notable changes to f13-rs. Versions follow [semver](https://semver.org); the supported game build is named in
each release.

## 0.1.0 (2026-10-12)

The first release: one map (Crystal Lake, small) from your own copy of *Friday the 13th: The Game* (supported
build: the Steam release current in October 2026; its executable's encrypted constants fall back to the January
2025 build's values).

### Setup
- `f13-installer`: pick your unpacked game folder; it reads the gameplay values, map, characters, sounds, HUD,
  intro and outro and particle effects from it, then Play. No Python, .NET or other tools needed.

### Counselors
- Vanessa with her stats, outfit and animations; walking, running, sprinting, crouching, combat stance, dodges,
  stumbles; stamina, fear (from the game's fear events), health and wounds.
- Items, cabinets and pockets; doors (lock, barricade), windows (climb, dive through), hiding spots with breath
  holding; the repair dial; first aid, weapons, the flare gun and shotgun, firecrackers, bear traps, the pocket knife.
- Escapes by car and by calling the police; radios; the map; spectating once dead or escaped.

### Jason (Part 2)
- A computer player that hunts by sight, sound and Sense; Morph, Stalk, Shift and rage; grabs, grab kills and placed
  kills with their kill cameras; throwing knives (picked up from the map) and traps; breaking doors and walls.
- Playable (`--play-jason`): the game's keys, his HUD, the Morph map, sound pings, door and knife prompts.

### World
- The game's lighting, tonemapper, colour grading, bloom, fog and rain; foliage; water; the power grid and fuse
  boxes; cars with working lights.

### Multiplayer
- A lobby with the game's Jason pick, networked matches, and voice chat.
