# f13-engine-poc

**A proof of concept for a custom engine to play *Friday the 13th: The Game*.**

The original game runs on Unreal Engine 4. This project tests whether it can be played on an engine of its own
instead: its rules, rendering, sound and networking rebuilt from scratch in Rust on [Bevy](https://bevyengine.org),
with the map, characters, sounds and gameplay values read from your own copy of the game. It ships no game files.

It's a proof of concept, not a finished game. Today it has one map (Crystal Lake, small) with its four cabin
layouts, every counselor and the Jasons, and online play with friends; expect missing features, rough edges and bugs.
It's an unofficial fan project, not affiliated with IllFonic or Gun Media.

## Install

You need your own copy of *Friday the 13th: The Game* with its files unpacked (its content as loose files under
`SummerCamp/Content`).

1. Download `f13-engine-poc-windows-x64.zip` from the [releases page](https://github.com/JLAgamer/f13-engine-poc/releases) and
   unzip it anywhere.
2. Run `f13-engine-poc-setup.exe` and pick your game folder (the one with `SummerCamp` in it).
3. Press **Install** (a few minutes: it reads everything it needs from your game), then **Play**.

Afterwards, start `f13-engine-poc.exe` directly. Run setup again (Reinstall) after a game update, or when a new f13-engine-poc
release asks for it.

**Requirements:** Windows 10 or 11 (64-bit), a DirectX 12 or Vulkan graphics card, about 12 GB free.

**Troubleshooting:** if Install fails, setup's log (below its buttons) says which step and why; a game folder that's
still packed is the usual cause. `f13-engine-poc.exe` writes its log to the console: start it from a terminal to see it.

**Uninstall:** delete the unzipped folder and `%USERPROFILE%\.f13-engine-poc` (your settings and profiles).

## Controls

| Counselor | |
|---|---|
| WASD, mouse | move (running), look |
| Shift / X / Ctrl | sprint / walk / crouch |
| E | use, pick up, open, hide |
| Left click | swing or use what you hold |
| C | combat stance (double-tap a direction to dodge; hold right click to block) |
| F | flashlight |
| M | map (when you carry one) |
| 1-3, G | pocket items, drop |
| T | push-to-talk |

Jason: left click attack, right click grab, C combat stance, R trap, 1 Morph, 2 Sense, 3 Shift, 4 Stalk, middle
click to aim and throw a knife, E to use or destroy. Holding a counselor, 1-4 pick a grab kill.

## Playing with friends

One player hosts, the others join. Open a terminal in the f13-engine-poc folder.

Host:

```bash
f13-engine-poc.exe --server 5000 --lobby --name Host
```

Everyone else (with the host's IP address):

```bash
f13-engine-poc.exe --connect 203.0.113.7:5000 --name Guest
```

- In the lobby, 1 / 2 / 3 say whether you'd rather be a counselor, have no preference, or be Jason; Enter readies you.
  Once everyone's ready, the game picks Jason as the real game does and the match starts.
- Voice chat is push-to-talk (hold T). In the lobby, P toggles push-to-talk and Shift + a player's number mutes them.
- Over the internet the host forwards **UDP ports 5000 and 5001** (the port given and the next one) to their PC. On
  the same network, use the host's local address.
- `--dedicated` hosts without playing. Everyone must run the same f13-engine-poc version.
- f13-engine-poc players only play each other; it never connects to the game's own servers.

## Building from source

You need [Rust](https://rustup.rs), the [.NET 10 SDK](https://dotnet.microsoft.com) and CMake (for voice chat's
libopus; on the PATH, or set `CMAKE`). Build, export from your game, and play:

```bash
cargo build --release -p f13-installer -p f13-game
```

```bash
dotnet build tools/f13-export -c Release
```

```bash
target/release/f13-installer --install "<path to F13Game>"
```

```bash
target/release/f13-game
```

`python tools/package.py` builds the release zip; `cargo test` runs the tests. The crates, the exporter, the game's
options and the developer tools are in [docs/DEVELOPING.md](docs/DEVELOPING.md).

## Credits

- Built with [Bevy](https://bevyengine.org), [lightyear](https://github.com/cBournhonesal/lightyear) (networking),
  [egui](https://github.com/emilk/egui) (setup), [cpal](https://github.com/RustAudio/cpal) and libopus (voice chat),
  and [CUE4Parse](https://github.com/FabianFG/CUE4Parse) (reading the game's files). Every bundled library's licence is
  in THIRD-PARTY-NOTICES.md in the release.
- *Friday the 13th: The Game* is by IllFonic and Gun Media. f13-engine-poc is an unofficial fan project, not affiliated
  with or endorsed by them; it ships none of their files and needs your own copy.
- Made with AI: the code, tools and documentation were written with Claude Code (Anthropic's Claude Opus models),
  directed and tested by the project's maintainer. No generated art or audio: everything you see and hear comes from
  your copy of the game.

## License

f13-engine-poc is licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
