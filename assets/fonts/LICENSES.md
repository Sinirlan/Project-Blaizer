# HUD fonts (Remaster "Modern" HUD, re-notes/09-remaster.md 9.21)

Embedded in the `motorhead` binary with `include_bytes!` (crates/motorview/src/ui_font.rs).

| File | Font | Author | Licence | Text |
|---|---|---|---|---|
| Teko-Bold.ttf | Teko Bold | Indian Type Foundry, 2014 | SIL Open Font License 1.1 (font name ID 13/14) | OFL-Teko.txt |
| Rajdhani-Medium.ttf | Rajdhani Medium | Indian Type Foundry, 2014 | SIL Open Font License 1.1 (font name ID 13/14) | OFL-Rajdhani.txt |
| Rajdhani-SemiBold.ttf, Rajdhani-Bold.ttf | Rajdhani SemiBold, Bold (Glass HUD menu, #294) | Indian Type Foundry, 2014 | SIL Open Font License 1.1 | OFL-Rajdhani.txt |
| Orbitron-ExtraBold.ttf | Orbitron ExtraBold (a static instance of `Orbitron[wght].ttf` at wght 800, fontTools instancer; Glass HUD menu, #294) | The Orbitron Project Authors (Matt McInerney), 2018 | SIL Open Font License 1.1 | OFL-Orbitron.txt |
| Motorhead.ttf | Motorhead | the project owner (FontForge, source `reference_materials/menu-overhaul/game-font/motorhead.sfd`) | own work, no restrictions | (none) |
| Overload.ttf | Overload | Ray Larabie, 1998/2004 | CC0 1.0 (Typodermic public-domain release, checked 2026-10-23) | LICENSE-Overload.txt |
| Audiowide-Regular.ttf | Audiowide (HUD clocks, LAP / POS values, gear digit, #305) | Brian J. Bonislawsky DBA Astigmatic (AOETI), 2012 | SIL Open Font License 1.1 (font name ID 13) | (none shipped; licence text is in the font's name table) |
| PromptFont.ttf | PromptFont (button prompts, keys, gamepads) | Shinmera (Yukari Hafner), based on Xolonium by Severin Meyer | SIL Open Font License 1.1 | OFL-PromptFont.txt |

PromptFont is compiled from `promptfont.sfd` with FontForge (`generate`); needs the credit line "PromptFont by Shinmera (Yukari Hafner), https://shinmera.com/promptfont" wherever the project is distributed.

## Lucide (motorview ribbon icons, DEBUG tools only)

| File | Font | Author | Licence | Text |
|---|---|---|---|---|
| lucide/lucide.ttf | Lucide icon font, lucide-static 1.61.0 (`font/lucide.ttf`) | Lucide Contributors (fork of Feather, Cole Bemis) | ISC | lucide/LICENSE |

Embedded in `motorview` (not `motorhead`) by crates/motorview/src/theme.rs; codepoints in crates/motorview/src/icons.rs.

## JetBrains Mono (motorview overlays and number fields, DEBUG tools only)

| File | Font | Author | Licence | Text |
|---|---|---|---|---|
| JetBrainsMono-Regular.ttf | JetBrains Mono Regular, release 2.304 | The JetBrains Mono Project Authors | SIL Open Font License 1.1 | OFL-JetBrainsMono.txt |

Embedded in `motorview` by crates/motorview/src/theme.rs as the first font of egui's Monospace family; drawn by crates/motorview/src/overlay.rs.
