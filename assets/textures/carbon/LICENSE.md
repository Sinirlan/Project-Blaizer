# Carbon fibre (Remaster generated car parts, re-notes/09-remaster.md 9.22 phase 5)

Embedded in the binaries with `include_bytes!` (crates/motorview/src/scene.rs `carbon_layers`).

| File | Source | Author | Licence |
|---|---|---|---|
| colour.png | Fabric004_1K-PNG_Color.png, box-downscaled to 256² | ambientCG (Fabric004) | CC0 1.0 |
| normal_rough.png | Fabric004_1K-PNG_NormalGL.png (RGB) + _Roughness.png (A), box-downscaled to 256² | ambientCG (Fabric004) | CC0 1.0 |

Made from the 1K PNG set in reference_materials/textures/carbon-fibre with PIL
(`Image.resize((256, 256), Image.BOX)`; the roughness merged as alpha).
