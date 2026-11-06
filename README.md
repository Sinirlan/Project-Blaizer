# Motorhead (Rust)

A reimplementation of **Motorhead** (Digital Illusions, 1998) for Linux and Windows 11.
It plays from a copy of the game that you own: **this repository contains no game data.**

- Classic mode follows the original's rendering; Remaster mode (F9 in the game) is a modern
  renderer over the same scene and the same physics.
- Crates: `motor-data` (loaders for the original file formats), `motor-sim` (car physics, race
  rules, AI), `motor-audio` (mixer), `motorview` (renderer and the game, `motorhead`; also a
  developer viewer, `motorview`), `motorinstall` (installs the game from your CD image).

## Install the game data

`motorinstall` builds the install folder from your own disc image: it unpacks the original
InstallShield cabinet, copies the movies and rips the CD music to FLAC (`Music/`). Started
without arguments it opens a small window (pick the image and the target folder, Install);
with arguments it runs in the terminal:

```sh
cargo run --release --bin motorinstall -- motorhead.cue ~/games/motorhead   # .cue/.bin: data + CD music
cargo run --release --bin motorinstall -- motorhead.iso ~/games/motorhead   # .iso: data only, no music
```

The source can also be a `.bin`/`.img` (its `.cue` beside it is used when present) or a folder
(a mounted disc or a copied CD). `--no-music` skips the music.

## Download

Ready-made `motorhead` and `motorinstall` for Linux and Windows (x86_64) are attached to each tag on the
Releases page, with `SHA256SUMS`. They contain no game data: you still need your own copy of the game.
The installer is also a download of its own (`motorinstall-<tag>-<platform>`).

The optional **Remaster pack** (`motorhead-remaster-<tag>.zip`, about 180 MB, with a `.sha256` file) holds the
Remaster mode's data: per-track fog, world fill, custom rims, HDRI skies, track pictures and the simulated
engine sound. Unzip it over the install folder (the one with `motor.exe`); it adds a `remaster/` folder.
Classic mode does not use it, and Remaster falls back to plainer defaults without it.

## Build and play

Rust 1.95 or newer. On Linux you need the ALSA and udev development packages
(`libasound2-dev`, `libudev-dev` on Debian/Ubuntu).

```sh
cargo build --release
cargo run --release --bin motorhead -- ~/games/motorhead
```

Or copy the `motorhead` program (`motorhead.exe` on Windows) next to `motor.exe` in the install
folder and start it there: with no folder argument it uses the folder it sits in. A folder
argument always wins.

`motorhead --version` prints the version. Settings are in `~/.config/motorhead/settings.cfg`
(Windows: `%APPDATA%\Motorhead\settings.cfg`).

## Other programs

```sh
cargo run --release -p motorview --bin motorview -- ~/games/motorhead --track 1 --car 1   # developer viewer
cargo run --release -p motor-sim --bin motorrace -- ~/games/motorhead --track 1            # headless AI race
```

Headless screenshot of any game screen: `motorhead ~/games/motorhead --shot SCREEN out.png [--remaster]`.

## About this repository

This repository is generated from the author's development repository, one commit per release.

## Licence

Copyright (C) 2026 Motorhead reimplementation contributors.

This program is free software: you can redistribute it and/or modify it under the terms of the
GNU Affero General Public License as published by the Free Software Foundation, either version 3
of the License, or (at your option) any later version (see `LICENSE`). Bundled assets keep their
own licences (see the `LICENSE` files under `assets/`). The licence covers this code only, not the
original game or its data, which stay the property of their owners.
