# Changelog

What changed in each release, newest first. Game data is never part of a release.

## Unreleased

## v0.9.1-alpha - 2026-11-06

### Gameplay

- AI: opponents drive much closer to the original. Their acceleration curve and top speed match it, they look ahead across the finish line, brake in the original's straight-line window, see cars just ahead when they overtake, and use the same lane-shift steering. Every car counts as the same size in traffic.
- Physics: handling is now computed at a fixed rate and the picture is interpolated between steps, so a car drives the same at any frame rate. One-shot sounds and gamepad rumble play once per hit.
- Physics: wall scrapes follow the original's rule (our own extra wall drag is gone), car-to-car contacts also push cars apart, and sideswipes drag the car like the original.
- Physics: jumps land like the original (the car is only levelled in the air above 30 degrees of tilt), and a car that ends up upside down freezes and is put back on its wheels after 2 seconds, as in the original.
- Physics: key and button braking ramps up from 70 % to full at the original's speed, and analog pedals have the original's 10 % boost.
- Physics: animated props that you hit lose their motion, and how they tumble follows the original.
- Cheats: the name plates of "ramlosa" and "h2o" follow the wobble of the picture; the Tron cheat has a Remaster neon grid with glowing edges, light walls and a cyan night sky.
- Replays record the physics step; replays made before this version still play.

### Graphics (Remaster)

- Trees and bushes can be drawn as 3D models instead of flat cards (Detail > Tree models). They take the ground's light and the track's tree look.
- Atlantika's sea is real water.
- Car bodies: small gaps and open seams closed on the Kobra X3, Python V12, Serpent, ASC II and Avalanche. The Serpent gets a new rear diffuser and centre exhaust, a straighter front arch and a sealed diffuser with no see-through slit. The Avalanche's front arches fit under the bonnet and its bonnet slot is closed.
- The Titan's grilles use half the triangles (20 000 instead of 42 000) with the same look.
- Headlight reflector bowls no longer show a staircase edge on slanted lenses.
- New app icon (a glass M) for the game window, the Windows programs and the Linux desktop entry. The installer and the viewer have their own icons.

### Menus and HUD

- Modern HUD: sector delta. At the checkpoints it shows the time difference for sectors 1 and 2, and a lap delta against your best earlier lap. The on-screen time has its own option.
- Demo play: sparks from the recorded spark slots, and mouse control of the demo controls (click buttons and chips, drag the bar and the diagram, hover bubbles, Glass pointer; the system arrow is hidden in races).
- League: in the middle of a season Classic skips the final league results screen, like the original.
- Classic minimap: other racers use the original's other-racer marker and you keep yours.

### Tools and downloads

- The installer (`motorinstall`) is now a separate download next to the game packages, and the Remaster data (fog, world fill, rims, skies, track pictures and the simulated engine sound) is an optional zip: unzip it over the install folder.