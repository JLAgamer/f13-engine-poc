# Roadmap

Open work only; what's done, with its references to the decomp, is in F13-decomp's MODLOG.

Base content (asset paths in the player's install; all gameplay values come from it, see README): Crystal Lake
(small) `Maps/CrystalLake_Small`, Vanessa `Characters/Counselors/Athlete`, Jason Part 2
`Characters/Killers/Jason/J2`, the Chevettes (`Blueprints/Vehicles/DriveableVehicles/Cars/Chevette`), the police.

## Todo
- [ ] Network sync audit: every player action from a client reaching the server, and every result showing on the
      clients (done so far: a client's Jason's Shift, Stalk, Sense and its highlights, Morph, knife pickups:
      --bot-net-abilities; MODLOG "A client's Jason: his abilities over the network")
- [ ] The lobby's rest: its counselor pick widget (the pick is C for now; the fourteen counselors play), a full
      8-player run
- [ ] Jason dying, its rest (MODLOG "Jason's death"): the kill's camera splines and prompt, a network run
- [ ] Tommy Jarvis: his intro at his start (FUN_14040fe60), HunterIntroComplete's pawn setup, a network bot run
- [ ] Voice chat: push-to-talk in the game's settings menu (no menus yet: P in the lobby stands in)
- [ ] Materials: foliage subsurface (its transmission is the deferred light pass's, whose code isn't in the shipped
      global shader cache: MODLOG "Foliage subsurface: blocked")
- [ ] Counselor model: the eye look against a capture of the game (it rolls the eyes 90 degrees from their bind:
      MODLOG "The eyes' look", "The counselor's face")
- [ ] Lighting: Enlighten GI (the map's indirect light: its precomputed systems and probe sets are Geomerics'
      runtime data, undocumented; the solver is in the exe)
- [ ] Lighting: the local lights' shadow caching and fade (which lights qualify is the game's; without the caching
      only the four nearest whose range reaches the camera render: MODLOG "Local lights' shadows")
- [ ] Lighting: the inverse-squared lights' unit (16/100^2 candela) is from engine memory, PLACEHOLDER until read
- [ ] The falling rain's simulation on the GPU as the game's (49 systems, about 23k sprites, simulated on the CPU:
      2.5 ms of the main world a frame; their draw is already on the GPU: MODLOG "The rain's draw")
- [ ] Post-processing and lighting: compare against the game's own screenshots (the user's cabin porch reference,
      2026-10-12: its wall lamp's spots light sharp bands up and down the logs)
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
