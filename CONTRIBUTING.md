# Contributing

Start with [ARCHITECTURE.md](ARCHITECTURE.md) for the map of the code.

## Building and testing

nex needs Rust 1.88 or newer. On Linux, also the development packages CI installs:

```sh
sudo apt-get install libxkbcommon-dev libwayland-dev libgl1-mesa-dev libx11-dev \
  libxcursor-dev libxrandr-dev libxi-dev libasound2-dev fontconfig
```

The checks CI runs (`.github/workflows/ci.yml`):

```sh
cargo build --locked
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
# nex-host ships to servers as a static binary:
cargo build --locked -p nex-host --release --target x86_64-unknown-linux-musl
```

CI also runs clippy on Windows, so the Windows platform layer keeps compiling.

## Code conventions

- **Module docs.** Every module root opens with a `//!` comment: what it is for, then how its
  files divide the work. Keep it current when you add or move a file.
- **Folders by role.** When a module outgrows one file, split it by role the way
  `file_explorer/` is (`fs/`, `input/`, `view/`, `tests/`), using `mod.rs` for the folder root.
- **Paths.** Prefer `crate::module::item` over chains of `super::`.
- **Visibility.** Inside `app/`, items shared between its submodules are `pub(in crate::app)`;
  helpers private to a folder are `pub(super)`; modules used across the crate (`ui`, `config`,
  `layout`, …) use plain `pub`.
- **OS-specific code** goes in `platform/` only.
- **Tests** sit next to the code (`#[cfg(test)] mod tests`), or in a `tests/` folder when a
  module has shared fixtures. Whole-app behavior is tested through egui in `app/tests/`.

## Releasing

1. Bump `version` in `Cargo.toml`, and move the `Unreleased` notes in
   [`CHANGELOG.md`](CHANGELOG.md) under the new version.
2. Commit, then tag and push:

   ```sh
   git tag v0.11.0 && git push origin v0.11.0
   ```

The [release workflow](.github/workflows/release.yml) builds x86_64 and aarch64 Linux binaries
(tarballs and `.deb`s) and a universal macOS build (`nex.app` zip and a tarball), and publishes a GitHub Release
with notes taken from the changelog.
