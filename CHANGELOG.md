# Changelog

What changed in each release, newest first. Game data is never part of a release.

## v0.9.2 - 2026-11-10

### Gameplay

- Keyboard steering: holding both steering keys steers right, as in the original.
- Gamepad: the stick keeps proportional steering, and its dead zone now defaults to 5 %.
- AI: a stuck opponent is only reset once it is out of your sight (150 m from the camera), so RedRock stalls play out like the original. The AI also sees across the finish line and passes cars that are level beside it.
- The camera you pick in a race is remembered. Holding the d-pad or stick repeats in menus. The Time Attack ghost stands steady on the grid.
- Attract demo: after 2 minutes idle on the main menu a 25-lap AI race starts; any key returns to the menu.
- League: after a race that is not the last of the season, Classic shows the standings table. The default name, the order of the New screens, and the way back to the main menu and the current league after the final follow the original.
- Hidden extras: RuhrStadt's club message with a hidden Credits page, and Atlantika's roof scroller, are in. The NeoCity sky road and Okkun's helicopter were checked against the track data and are both there.

### Graphics (Classic)

- Classic is closer to the original's Direct3D look: car and scene-object shading, depth fog on the world and on objects (distant things fade to silhouettes), larger world halos, and the original's lens flares. Cars and loose props are lit by the track's centreline tint.
- Textures are scaled up smoothly instead of blocky, like the original's linear filtering.
- Ghost cars (Ghost mode, the Time Attack ghost and the casper cheat) look like the original's remote cars: a soft additive glow with no shadow or halos.
- Widescreen: the view keeps the original's height of view and shows more to the sides (106° across at 16:9) instead of stretching.
- In-car views no longer draw the shadow of the car you are in.

### Graphics (Remaster)

- Widescreen works the same way as in Classic. The main view keeps the original's height of view, and the Cine cameras keep their field of view.
- Olympos: the whole far part of the sea is water (waves, reflections, shore foam and fog).
- Car bodies: the Kobra X3 gets rounded front fenders, a domed bonnet and nose, rear quarters and honeycomb intakes. The Serpent gets a smoother front arch and a tail with level lamp and grille edges and louvres. The ASC gets faired-in lamps, a filled and domed bonnet, and a tail with a diffuser and outlet.
- Headlight reflector bowls follow the lens edge at folds on the Avalanche and the ASC.

### Audio

- Engine echo follows the track data like the original.

### Menus and HUD

- Classic menus: every screen slides in from the original's start positions and slides out before the next one comes in, with the original's per-row effects. The title and box and the halftone dots panel match the original on every screen. Fixed glitches in the slot drawer, the title/box flip-over and value rolls.
- New Lap Analysis screen: compare two laps (your ghosts, replays or demos) with driver cards, a track map coloured by the faster stretch, a speed chart, a time delta and a lap picker. Open it with ANALYSE on the Ghost screen, or browse and drag in files from outside the game.
- Proximity arrows no longer flicker along their axis.

### Tools

- Car models can be loaded from .glb files (body, wheels per corner, lamp halos), and the generated car body can be exported for editing in Blender.


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
