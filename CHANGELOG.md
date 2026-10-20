# Changelog

All notable changes to f13recomp. Versions follow [semver](https://semver.org); the supported game build is named in
each release.

## Unreleased

- The project is renamed f13recomp (was Lakeside): f13recomp.exe, f13recomp-setup.exe, tools/f13recomp-export. The
  settings folder `.f13-rs` in your user folder moves to `.f13recomp` by itself on the first start.

## 0.2.1 (2026-10-15)

Same map, game build and export as 0.2.0: no need to install again; replace the files with this zip's.

### Network play
- Jason's pick no longer restarts anyone: each player's Jason, grab kills and weapon go with them, and the match's
  Jason loads as it starts.
- A player who leaves is gone from the lobby at once.
- Clients' movement no longer stutters.
- A client playing Jason gets everything the host's Jason does: the knife's crosshair and zoom, the knife pickup's
  camera, the kill cameras, the Shift and Stalk bars, his stun's screen effects, and the door and placed kill
  prompts.
- A client's counselor gets the door, window and drawer cameras, and no longer takes over the host's view with them;
  clients hear windows opening and closing, barricades and diving through glass.

### Gameplay
- Sense lights a counselor's whole cabin, with the red light out of its windows and cracks.
- Items, doors and the other things a counselor can use show their prompt from a few metres away, as in the game.
- Jason's swings no longer hit through walls; thrown knives stop spinning where they stick; only his own mask
  drops.
- The Head Stomp grab kill kills (it used to leave her alive).

### Fixes
- Sound comes back after the audio device changes (a headset, tabbing out), voice chat too.
- Jason's HUD glow (the unlock notice, the Morph cursor) as bright as the game's.
- A client could crash when a voice line ended as it started.

## 0.2.0 (2026-10-14)

Same map and supported game build as 0.1.0. Install again: setup now also exports every counselor, every Jason
from Part 2 to Part 8, their weapons and the menus.

### Characters
- Every counselor playable (the female ones on Vanessa's skeleton, the male ones on their own animations), with
  their hair, faces, stats and voices; Tommy Jarvis comes when the CB radio calls him.
- Jason Parts 2 to 8, each with his own strengths and weaknesses, ability times, grip and mask; every grab kill the
  game has (62) with their animations, cameras and music; every weapon he may carry, each swinging its own way.
- Jason can be killed: his health and mask, Pamela's sweater and its stun, and his death scene.

### Menus
- The game's lobby (players, ready bar, pings, voice, map), and back to it after a match with its results, badges
  and scoreboard.
- The counselor menu with its 3D preview, the wardrobe, emotes (and the emote wheel in a match) and perks (their
  effects in the match).
- The Jason menu with its 3D preview: pick the Jason, his grab kills and his weapon; the lobby starts again with the
  Jason picked.
- The audio settings; the lobby and its menus by the mouse.

### Gameplay
- Characters no longer walk through each other; interaction prompts for doors, windows and walls (Jason's too).
- Blood, gore and dismemberment on counselors; Jason's bloodied weapon; the kills' screen blood.
- Network: hiding, repairs, cars, the police, traps, Sense, Morph, Jason's death and the menus' picks all work for
  players on other machines; push-to-talk voice with the game's ranges.

### Fixes
- Counselors' faces in the menus (eyes hidden, Sheldon's face missing, Tiffany's make-up); the skins' normal maps;
  Jason's faint HUD effects; the lights' brightness unit.

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
