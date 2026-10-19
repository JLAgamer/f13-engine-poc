# Roadmap

Open work only; what's done, with its references to the decomp, is in F13-decomp's MODLOG.

Base content (asset paths in the player's install; all gameplay values come from it, see README): Crystal Lake
(small) `Maps/CrystalLake_Small`, Vanessa `Characters/Counselors/Athlete`, Jason Part 2
`Characters/Killers/Jason/J2`, the Chevettes (`Blueprints/Vehicles/DriveableVehicles/Cars/Chevette`), the police.

## Todo
Lighting first (the user, 2026-10-14):
- [ ] Lighting: Enlighten GI (on by default; F13_ENLIGHTEN=0 off): left are the game's own versions of what's fitted
      or guessed (the lights' input: albedo x E, matched to the user's shots, the UE integration's fill unread; the
      local lights' visibility: one ray per point; the landscape's UVs: fitted; the foliage's: one point per instance;
      the probes' interpolation and per-pixel clipmap)
- [ ] Lighting: the local lights' shadow caching and fade: whole-map caching gains nothing (the shadowed lights are near
      her, so always dirty: tried, MODLOG "Shadow caching: tried and reverted"); it needs UE's static / movable split, two
      shadow views a face in a reworked bevy_pbr
- [ ] Lighting against the user's 2026-10-17 shots (MODLOG "Too dark overall", "The height fog along the ray"): with the
      game's own fog the unlit views are 30-40% short again (Rod's logs, the intro's background; Zane's 10%): the
      fill left is Enlighten's (its input scale fitted, PLACEHOLDER); the light shafts (the fog's occlusion, the moon's bloom: MODLOG "Light shafts") not drawn;
      Chad's hair in the intro still mid brown, near black in the game (its permutation's colour now as compiled; the
      rest unread: UE's hair shading against Bevy's standard lighting?)
- [ ] The lobby's mouse controls: all wired (lobby.rs lobby_clicks, ui.rs entry_hover: buttons, list entries, the emote
      wheel, hover looks, sounds); to try with a real mouse (the user)
- [ ] The lobby's rest: a full 8-player run (needs more machines: at most 2 instances run here)
- [ ] Materials: foliage subsurface (its transmission is the deferred light pass's, whose code isn't in the shipped
      global shader cache: MODLOG "Foliage subsurface: blocked")
- [ ] Counselor model: the eye look against a capture of the game (it rolls the eyes 90 degrees from their bind:
      MODLOG "The eyes' look", "The counselor's face")
- [ ] Melee impacts, the approximations left (MODLOG "All melee impacts", "Jason's swing recoiling"): Jason's weapon
      sweep (a chest line along his facing now: its penetration and distance stand in for the hit's), where a hit
      lands on a character (a fixed point on the chest now)
- [ ] Lighting: Enlighten's remaining stand-ins: the lamps' visibility (a ray per point for the game's shadow-map
      test), the landscape's per-component UV scale and bias (fitted from the data; set per draw at run time)
- [ ] Optimize performance further (2026-10-15, a counselor at her start, --perf --features profile: 8.0 ms a frame,
      from 13.4 that day; render world 7.3 ms, mostly encoding 4.7; main world 4.5, PostUpdate 2.9; particles 0.9.
      Left: the remaining non-bindless materials (masters, terrain, water, characters), the moon's cascades ~3 ms,
      the game's cached whole-scene shadows: MODLOG "Performance: bindless materials")
- [ ] Music: Jason's own music (JasonsMusic, for one spectating him: MODLOG "Jason's music, read further"; Jason is
      spectated only in private matches, so it waits for those)
- [ ] More content, one at a time: maps (with their boat and other escapes), Jasons (J2-J8 play with every grab kill
      and weapon; still: J9, JTS1, JX, J9's battle axe textures, the kills' limb mesh swaps, exported in
      anim_gore.json and BodyParts' MeshSwaps, and the weapons' combat idles), Jason skins, counselors, perks, skins,
      emotes, a main menu
- [ ] The Jason and kill menus by a real mouse and keys (Q opens the weapons, 1-4 the grab kill slots: PLACEHOLDER keys)

- [ ] Playtest (2026-10-17): Jason's boots about 10 cm in the ground (the game's own data: his mesh 102 cm under an
      88 cm capsule's centre, nothing found resizing it; to compare in the game). (Done: counselors' feet, the context
      kill prompts, J7's machete.)

## Later
- All unlockables and perks available by default
- Server browser using Steamworks
- Mod support: a mod repository that clients pull from; modded lobbies are labelled in the browser and
  joining one downloads its mods
- Configurable community backends: progression, allowed mods, which servers are listed, player reports
  (for example vanilla-only, cheater-banning communities as well as the default "anything goes")

Mod ideas: custom game modes (infection), larger lobbies, custom counselors and Jasons, custom animations.
