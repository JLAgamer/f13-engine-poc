# Roadmap

Open work only; what's done, with its references to the decomp, is in F13-decomp's MODLOG.

Base content (asset paths in the player's install; all gameplay values come from it, see README): Crystal Lake
(small) `Maps/CrystalLake_Small`, Vanessa `Characters/Counselors/Athlete`, Jason Part 2
`Characters/Killers/Jason/J2`, the Chevettes (`Blueprints/Vehicles/DriveableVehicles/Cars/Chevette`), the police.

## Todo
Lighting first (the user, 2026-10-14):
- [ ] Lighting: Enlighten GI (the map's indirect light: its precomputed systems and probe sets are Geomerics'
      runtime data, undocumented; the solver is in the exe)
- [ ] Lighting: the local lights' shadow caching and fade (which lights qualify is the game's; without the caching
      only the four nearest whose range reaches the camera render: MODLOG "Local lights' shadows")
- [ ] Lighting: the inverse-squared lights' unit (now 1/100^2 candela from the game's shaders and light proxy): read the
      deferred light uniform's fill to close it (MODLOG "The local lights' unit, the shaders")
- [ ] Post-processing and lighting: compare against the game's own screenshots (the user's cabin porch reference,
      2026-10-12: its wall lamp's spots light sharp bands up and down the logs)
- [ ] Jason's HUD: the ability unlock and Morph cursor's shader (M_UI_JasonHud_01, shaders/additive.wgsl) draws a little
      too faint (the user, 2026-10-14): likely the blend's space: Slate adds a UI material's output to the gamma-encoded
      image, f13-rs adds the decoded value to the linear one (the same over black, fainter over anything lit: the
      Morph map); needs the HUD's additive brushes drawn in a gamma-space layer
- [ ] The lobby's mouse controls (the user, 2026-10-14): its buttons click now (lobby.rs lobby_clicks: they press the
      keys their menus take); still to do: clicking the menus' list entries (perks, outfits, swatches, emotes) and the
      buttons' hovered / pressed looks (their Button styles)
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
