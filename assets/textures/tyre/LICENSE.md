# Tyre rubber (Remaster generated wheels, re-notes/09-remaster.md 9.20)

Embedded in the binaries with `include_bytes!` (crates/motorview/src/scene.rs `tyre()`).

| File | Source | Author | Licence |
|---|---|---|---|
| colour.png | plastic_0022_color_2k.jpg x plastic_0022_ao_2k.jpg, box-downscaled to 256² | TextureCan, "Clean Car Tire Rubber Texture (Plastic 0022)" | TextureCan free licence [U: check the terms on texturecan.com before publishing] |
| normal_rough.png | plastic_0022_normal_opengl_2k.jpg (RGB) + plastic_0022_roughness_2k.jpg (A), box-downscaled to 256² | TextureCan | as above |

Made from the 2K set in reference_materials/textures/tire with PIL (`Image.resize((256, 256), Image.BOX)`).
The height and DirectX normal maps are not used.
