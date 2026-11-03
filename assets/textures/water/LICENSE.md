# Water detail textures (Remaster water, re-notes/completed/16-water.md)

Embedded in the binaries with `include_bytes!` (crates/motorview/src/render.rs `water_textures`).

| File | Source | Licence |
|---|---|---|
| normal.png | `textures/water_normal.png` (1024², tangent-space normals), box-downscaled to 512², RGBA (A = 255) | MIT (below) |
| foam.png | R = `textures/foam.png` (1024², grey) box-downscaled to 512²; G = red channel of `textures/foam_shore.png` (1024×512) resized to 512² (stretched 2:1, still tiles); B = 0, A = 255 | MIT (below) |

Source: https://github.com/tuxalin/water-shader (commit on `master`, fetched 2026-10-26), made with PIL.

```
MIT License

Copyright (c) 2017 tuxalin

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
