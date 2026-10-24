<p align="center">
  <img src="assets/icon/w2r-mod-loader-256.png" alt="W2R Mod Loader icon" width="160"><br>
  <sub>Icon by Maxx444</sub>
</p>

# W2R Mod Loader

A mod loader and plugin platform for Warcraft II Remastered (game build 1.0.2.2818, 32-bit). Players
install mods with the **W2R Mod Loader** app; developers write plugins against its C ABI.

- **Host** (`w2r-core`): a `version.dll` proxy that loads signed-off native plugins into the game,
  and carries the features ported from [wc2r-plus](https://github.com/Luxray997/wc2r-plus) (by Luxray997): the
  lobby handshake, chat history, lobby policy, map download and sharing, and the enhanced Create
  Multiplayer Game and lobby screens. Plugins get a small C ABI ([docs/abi.md](docs/abi.md)).
- **Manager** (`w2r-loader-lib`, the `w2r-loader` CLI and the `w2r-loader-gui` egui app): reads a plugin index,
  downloads, checks hashes and installs plugins, and keeps their enabled and approved state.
- **Plugins** live in a separate repository, [`w2r-plugins`](https://github.com/hastinbe/w2r-plugins),
  with CI that builds the packages and publishes the index.

## Warning

This is not an official Blizzard mod. It replaces the game's `version.dll` and loads native plugin
DLLs into the game, so it may be flagged by Blizzard's anti-cheat. No bans have been reported for
wc2r-plus, which it is based on, but that is no guarantee. Proceed at your own risk, and only install
plugins you trust: native plugins are not sandboxed (see [docs/threat-model.md](docs/threat-model.md)).

## Status

Phases 0 to 5 are written and tested; Phase 6 (the plugin repository and CI) is written.
[docs/PLAN.md](docs/PLAN.md) has the per-phase status and what is still open. Not done yet:

- The plugin repository is still private, so the default index URL does not resolve yet.
- No plugin SDK crate (`w2r-sdk`); plugins use the `w2r-abi` bindings or the C header directly.
- The egui app has been tested through its job layer, not clicked through by a person.
- Features that need two machines (lobby policy, map download between two players, the lobby screen
  with a second player) are only partly verified in the game.

## Layout

```
crates/
  w2r-abi/       C header and Rust bindings for the plugin ABI
  w2r-formats/   strict parsers shared by host and manager: JSON, SemVer, plugin.json, state.json
  w2r-core/      the in-game host (32-bit cdylib, version.dll proxy)
  w2r-loader-lib/     manager logic: index, download, verify, install
  w2r-loader-cli/     the `w2r-loader` command
  w2r-loader-gui/     the egui manager
  test-plugins/  fixture DLLs for the host's tests
docs/            specs, plan, threat model, per-feature notes
tools/           spec reference implementations, generators, the dev installer
```

## Specs

- [docs/PLAN.md](docs/PLAN.md), [docs/threat-model.md](docs/threat-model.md), [docs/layout.md](docs/layout.md)
- [docs/abi.md](docs/abi.md) and [crates/w2r-abi/include/w2r_abi.h](crates/w2r-abi/include/w2r_abi.h): plugin ABI
- [docs/manifest.md](docs/manifest.md): `plugin.json`
- [docs/index.md](docs/index.md): plugin index
- [docs/load-policy.md](docs/load-policy.md): when the host loads a plugin, and the simulation fingerprint
- [docs/settings-screen.md](docs/settings-screen.md): the options screen, where the game's, the host's and every plugin's settings live
- [docs/manager.md](docs/manager.md): the manager library, CLI and GUI
- [docs/plugin-dev-setup.md](docs/plugin-dev-setup.md): set up a PC to build and test your own plugins (rustup, MSVC, the 32-bit target, local install)
- [docs/peer-protocol.md](docs/peer-protocol.md): the host's own peer messages (the handshake), wire-compatible with wc2r-plus
- Features: [chat-plugin.md](docs/chat-plugin.md), [lobby-policy.md](docs/lobby-policy.md),
  [map-download.md](docs/map-download.md), [create-screen.md](docs/create-screen.md),
  [lobby-screen.md](docs/lobby-screen.md)
- [docs/vectors/](docs/vectors/): test vectors

Run `bash tools/run_all.sh` to check the vectors and the C header.

## Building

The game is 32-bit, so the host is built for `i686-pc-windows-msvc` (rustup target plus the VS
Build Tools C++ workload):

```
cargo test --workspace --target i686-pc-windows-msvc
cargo build --release -p w2r-core --target i686-pc-windows-msvc
```

The result, `target/i686-pc-windows-msvc/release/w2r_core.dll`, goes into the game's `x86`
folder as `version.dll`. The host writes `w2r.log` under `Saved Games\Warcraft2Remastered\w2r\logs`
(or `%W2R_ROOT%\logs`). `cargo test` without `--target` also works for the pure-Rust parts. The
manager (`cargo run -p w2r-loader-cli`, `cargo run -p w2r-loader-gui`) is an ordinary 64-bit build.

`tools\install_dev.ps1` does the build and install and sets up the basic fixture plugin
under Saved Games (`-WithChat` adds the chat plugin, `-WithCrasher` the crashing fixture,
`-Uninstall` removes the DLL). Set `PYTHON=/path/to/python.exe` if `python3` is missing (Windows). The
32-bit header check uses `gcc -m32` or, on Windows, MSVC located via `vswhere`.

## Credits

Large parts of the host and the whole chat plugin are ports of wc2r-plus by Luxray997 (MIT); see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Credits

The icon is by Maxx444 (see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)).
