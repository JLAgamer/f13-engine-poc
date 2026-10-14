# Roadmap

Open work only; what's done, with its references to the decomp, is in F13-decomp's MODLOG.

Base content (asset paths in the player's install; all gameplay values come from it, see README): Crystal Lake
(small) `Maps/CrystalLake_Small`, Vanessa `Characters/Counselors/Athlete`, Jason Part 2
`Characters/Killers/Jason/J2`, the Chevettes (`Blueprints/Vehicles/DriveableVehicles/Cars/Chevette`), the police.

## Todo
Lighting first (the user, 2026-10-14):
- [ ] Lighting: Enlighten GI (the map's indirect light: its precomputed systems and probe sets are Geomerics'
      runtime data, undocumented; the solver is in the exe)
- [ ] Lighting: the local lights' shadow caching and fade: whole-map caching gains nothing (the shadowed lights are near
      her, so always dirty: tried, MODLOG "Shadow caching: tried and reverted"); it needs UE's static / movable split, two
      shadow views a face in a reworked bevy_pbr
- [ ] Post-processing and lighting: compare against the game's own screenshots (the user's cabin porch reference,
      2026-10-12: its wall lamp's spots light sharp bands up and down the logs)
- [ ] The lobby's mouse controls: all wired (lobby.rs lobby_clicks, ui.rs entry_hover: buttons, list entries, the emote
      wheel, hover looks, sounds); to try with a real mouse (the user)
- [ ] The lobby's rest: a full 8-player run (needs more machines: at most 2 instances run here)
- [ ] Materials: foliage subsurface (its transmission is the deferred light pass's, whose code isn't in the shipped
      global shader cache: MODLOG "Foliage subsurface: blocked")
- [ ] Counselor model: the eye look against a capture of the game (it rolls the eyes 90 degrees from their bind:
      MODLOG "The eyes' look", "The counselor's face")
- [ ] Optimize performance (2026-10-13, a counselor at her start, --perf --features profile: 11.4 ms a frame, the render
      world's CPU the critical path: encoding 4.4 ms, submit 1.6, preprocess bind groups 1.3, the shadow phases'
      instance buffers 1.3; main world 3.9, PostUpdate 2.5; particles 0.9)
- [ ] Music: Jason's own music (JasonsMusic, for one spectating him: MODLOG "Jason's music, read further"; Jason is
      spectated only in private matches, so it waits for those)
- [ ] More content, one at a time: maps (with their boat and other escapes), Jasons (with their weapons' and hand
      kills: the axe, fire axe and hand kills' limb mesh swaps, exported in anim_gore.json and BodyParts' MeshSwaps),
      counselors, perks, skins, emotes, a main menu

## Later
- All unlockables and perks available by default
- Server browser using Steamworks
- Mod support: a mod repository that clients pull from; modded lobbies are labelled in the browser and
  joining one downloads its mods
- Configurable community backends: progression, allowed mods, which servers are listed, player reports
  (for example vanilla-only, cheater-banning communities as well as the default "anything goes")

Mod ideas: custom game modes (infection), larger lobbies, custom counselors and Jasons, custom animations.
