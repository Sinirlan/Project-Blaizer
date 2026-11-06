//! The graphics keys of `settings.cfg` [design choice]: the game (front.rs) and motorview read them
//! through the same parser, so a track calibrated in motorview (fog, sky, light) looks the same in
//! the game. Display keys (Monitor, Fullscreen, ScreenRatio, Resolution, VSync, FrameLimit) are
//! the game window's own and are not handled here.

use crate::render::{parse_aa_strength, CarDetail, CarShading, FogStyle, GrassLevel, LightShadows, PostAaKind, RemasterView, RenderAa, RenderAaKind, RenderMode, ShadowQuality, Tonemap, View};
use crate::revmeter::RevMeterTune;
use std::path::PathBuf;
use std::str::SplitWhitespace;

/// REMASTER halo style (`RemasterView::smooth_halos`): the original sprites or the analytic shape
pub const HALO_STYLES: [&str; 2] = ["Classic", "Smooth"];
pub const SHADOW_STYLES: [&str; 2] = ["Classic", "Real"];
/// REMASTER race HUD (`RemasterView::modern_hud`): the bitmaps scaled up or the modern HUD (9.21)
pub const HUD_STYLES: [&str; 2] = ["Classic", "Modern"];
/// REMASTER MotionBlur (`RemasterView::velocity_blur`): the original's frame blend or the
/// velocity blur
pub const BLUR_STYLES: [&str; 2] = ["Classic", "Velocity"];
/// REMASTER skid marks (`RemasterView::skids_detailed`): the original's flat strips or the
/// tread-textured ones
pub const SKID_STYLES: [&str; 2] = ["Classic", "Detailed"];
/// REMASTER car models (`RemasterView::generated_cars`, 09-remaster 9.20)
pub const CAR_MODELS: [&str; 2] = ["Original", "Generated"];

/// `Off` / `Nx` back to a level; bare numbers too (0 = off).
pub fn parse_level(v: &str) -> Option<u32> {
    if v.eq_ignore_ascii_case("off") {
        return Some(1);
    }
    v.trim_end_matches(['x', 'X']).parse::<u32>().ok().map(|n| n.max(1))
}

pub fn pick(v: &str, names: &[&str]) -> Option<usize> {
    names.iter().position(|n| n.eq_ignore_ascii_case(v))
}

/// Apply one `Key value` line if it is a graphics key of the `View`; false = not one of ours
/// (`rest` is the line after the value). Unknown values keep the current setting, ranges are clamped.
pub fn apply<'a>(view: &mut View, k: &str, v: &'a str, rest: &mut SplitWhitespace<'a>) -> bool {
    let n = v.parse::<i64>().ok();
    let u = |max: i64| n.map(|x| x.clamp(0, max) as u32);
    let on = n.map(|x| x != 0);
    let r = &mut view.remaster;
    match k.to_ascii_lowercase().as_str() {
        "shadows" => view.shadows = on.unwrap_or(view.shadows),
        "halos" => view.halos = on.unwrap_or(view.halos),
        "fog" => view.fog = on.unwrap_or(view.fog),
        // Enhanced was a car shading before it moved to the Remaster options
        "carshading" if v.eq_ignore_ascii_case("enhanced") => (view.car_shading, r.enhanced_paint) = (CarShading::Reflection, true),
        "carshading" => view.car_shading = CarShading::parse(v).unwrap_or(view.car_shading),
        "cardetail" => view.car_detail = CarDetail::parse(v).unwrap_or(view.car_detail),
        // older settings files
        "reflections" => view.car_shading = if on.unwrap_or(true) { CarShading::Reflection } else { CarShading::Gouraud },
        "viewdistance" => view.distance = n.map_or(view.distance, |x| x.clamp(30, (View::MAX_DISTANCE * 100.0) as i64) as f32 / 100.0),
        // Brightness R G B in percent, capped at 200 (0x418120); one value here
        "brightness" => view.brightness = n.map_or(view.brightness, |x| x.clamp(50, 200) as f32 / 100.0),
        "skyimage" => view.sky_image = on.unwrap_or(view.sky_image),
        "motionblur" => view.motion_blur = on.unwrap_or(view.motion_blur),
        "lensflares" => view.lens_flares = on.unwrap_or(view.lens_flares),
        "sparks" => view.sparks = on.unwrap_or(view.sparks),
        "smoke" => view.smoke = on.unwrap_or(view.smoke),
        "graphicsmode" => view.mode = RenderMode::parse(v).unwrap_or(view.mode),
        "remastercarpaint" => r.enhanced_paint = on.unwrap_or(r.enhanced_paint),
        // DynamicLights: older settings files
        "remasterdynamiclights" | "dynamiclights" => r.dynamic_lights = on.unwrap_or(r.dynamic_lights),
        // "Off" / "Nx"; a bare 0 / 1 is the first version's on/off switch (1 = on)
        // (older files: RemasterMSAA, the sample count; RenderAA replaces it)
        "remastermsaa" => {
            let d = RemasterView::default().msaa();
            let x = parse_level(v).map(|x| if v == "1" { d } else { x });
            if let Some(x) = x.filter(|x| RemasterView::MSAA_LEVELS.contains(x)) {
                r.render_aa = RenderAa::msaa(x);
            }
        }
        "remasterrenderaa" => r.render_aa.kind = RenderAaKind::parse(v).unwrap_or(r.render_aa.kind),
        "remasterrenderaastrength" => r.render_aa.strength = parse_aa_strength(v).unwrap_or(r.render_aa.strength),
        "remasterpostaa" => r.post_aa.kind = PostAaKind::parse(v).unwrap_or(r.post_aa.kind),
        "remasterpostaastrength" => r.post_aa.strength = parse_aa_strength(v).unwrap_or(r.post_aa.strength),
        "remasteranisotropy" => {
            let x = parse_level(v);
            r.anisotropy = x.and_then(|x| RemasterView::ANISOTROPY_LEVELS.into_iter().find(|&l| l as u32 == x)).unwrap_or(r.anisotropy);
        }
        "remasterpixellight" => r.pixel_light = on.unwrap_or(r.pixel_light),
        "remasterskyhdri" => r.sky_hdri = on.unwrap_or(r.sky_hdri),
        "remasterfogstyle" => r.fog_style = FogStyle::parse(v).unwrap_or(r.fog_style),
        "remasterpixelfog" => r.pixel_fog = on.unwrap_or(r.pixel_fog),
        "remastersigns" => r.retro_signs = on.unwrap_or(r.retro_signs),
        "remasterworldfill" => r.world_fill = on.unwrap_or(r.world_fill),
        "remastertreemodels" => r.tree_models = on.unwrap_or(r.tree_models),
        "remasterwater" => r.water = on.unwrap_or(r.water),
        "remasterwaterreflections" => r.water_ssr = on.unwrap_or(r.water_ssr),
        "remasterlocallight" => r.local_light = on.unwrap_or(r.local_light),
        "remastertexturepack" => r.texture_pack = on.unwrap_or(r.texture_pack),
        "remastergrass" => r.grass = GrassLevel::parse(v).unwrap_or(r.grass),
        "remastercarloddistance" => {
            let (lo, hi) = RemasterView::CAR_LOD_DISTANCE;
            r.car_lod_distance = n.map_or(r.car_lod_distance, |x| x.clamp(lo as i64, hi as i64) as u32);
        }
        "remastergrassrange" => {
            let (lo, hi) = RemasterView::GRASS_RANGE;
            r.grass_range = n.map_or(r.grass_range, |x| x.clamp(lo as i64, hi as i64) as u32);
        }
        "remasterfogstrength" => {
            let (lo, hi) = RemasterView::FOG_STRENGTH;
            r.fog_strength = n.map_or(r.fog_strength, |x| x.clamp(lo as i64, hi as i64) as u32);
        }
        "remasterclearcoat" => r.clear_coat = on.unwrap_or(r.clear_coat),
        "remastercolourgrade" => r.grade = u(100).unwrap_or(r.grade),
        "remastervignette" => r.vignette = u(100).unwrap_or(r.vignette),
        "remastergrain" => r.grain = u(100).unwrap_or(r.grain),
        "remasterhdr" => r.hdr = on.unwrap_or(r.hdr),
        "remastertonemap" => r.tonemap = Tonemap::parse(v).unwrap_or(r.tonemap),
        "remasterexposure" => {
            let (lo, hi) = RemasterView::EXPOSURE_RANGE;
            r.exposure = n.map_or(r.exposure, |x| x.clamp(lo as i64, hi as i64) as u32);
        }
        "remasterbloom" => r.bloom = u(100).unwrap_or(r.bloom),
        "remasterssao" => r.ssao = u(100).unwrap_or(r.ssao),
        "remasterflares" => r.smooth_flares = pick(v, &HALO_STYLES).map_or(r.smooth_flares, |i| i == 1),
        "remastershadows" => r.car_shadows = pick(v, &SHADOW_STYLES).map_or(r.car_shadows, |i| i == 1),
        "remasterhalos" => r.smooth_halos = pick(v, &HALO_STYLES).map_or(r.smooth_halos, |i| i == 1),
        "remastermotionblur" => r.velocity_blur = pick(v, &BLUR_STYLES).map_or(r.velocity_blur, |i| i == 1),
        "remasterskidmarks" => r.skids_detailed = pick(v, &SKID_STYLES).map_or(r.skids_detailed, |i| i == 1),
        "remasterlightshadows" => r.light_shadows = LightShadows::parse(v).unwrap_or(r.light_shadows),
        "remastershadowquality" => r.shadow_quality = ShadowQuality::parse(v).unwrap_or(r.shadow_quality),
        // the map size of older settings files (before the quality option)
        "remasterlightshadowsize" => r.shadow_quality = n.map_or(r.shadow_quality, |x| ShadowQuality::from_size(x.max(0) as u32)),
        "remastercarmodels" => r.generated_cars = pick(v, &CAR_MODELS).map_or(r.generated_cars, |i| i == 1),
        // a saved "Sharp" (9.12, superseded) loads as Modern
        "remastermenu" => r.modern_menu = pick(v, &HUD_STYLES).map_or(r.modern_menu, |i| i == 1),
        "remasterhud" => r.modern_hud = if v.eq_ignore_ascii_case("sharp") { true } else { pick(v, &HUD_STYLES).map_or(r.modern_hud, |i| i == 1) },
        // glow, inner glow, glass alpha, spec, bloom, thickness, depth, roundness (revmeter.rs)
        "remasterrevmeter" => {
            let vals: Vec<u32> = std::iter::once(v).chain(rest.by_ref()).filter_map(|x| x.parse().ok()).collect();
            if let Ok(a) = <[u32; RevMeterTune::FIELDS]>::try_from(vals.as_slice()) {
                r.rev_meter = RevMeterTune::from_array(a);
            }
        }
        _ => return false,
    }
    true
}

/// What motorview takes from `settings.cfg` besides the `View`.
pub struct Loaded {
    pub path: PathBuf,
    /// `SkidMarks` (a pass of the race, not part of the `View`)
    pub skid_marks: bool,
}

/// The game's `settings.cfg` over `view`; `None` when there is none (the view is untouched).
pub fn load(view: &mut View) -> Option<Loaded> {
    let path = crate::ghost::config_dir()?.join("settings.cfg");
    let text = std::fs::read_to_string(&path).ok()?;
    let mut skid_marks = true;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(k), Some(v)) = (it.next(), it.next()) else { continue };
        if k.eq_ignore_ascii_case("skidmarks") {
            skid_marks = v != "0";
        } else {
            apply(view, k, v, &mut it);
        }
    }
    Some(Loaded { path, skid_marks })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> View {
        View { brightness: 1.0, fog: true, show_line: false, shadows: true, halos: true, car_shading: CarShading::Reflection, car_detail: CarDetail::High, distance: 1.0, fov_x: crate::render::FOV_X.get(), sky_image: true, motion_blur: false, lens_flares: true, smoke: true, sparks: true, mode: RenderMode::Classic, remaster: RemasterView::default(), debug: Default::default() }
    }

    #[test]
    fn graphics_keys_only() {
        let mut v = view();
        let text = "GraphicsMode Remaster\nRemasterFogStyle Volumetric\nViewDistance 250\nRemasterExposure 120\nRemasterRevMeter 100 90 80 70 60 50 41 210\nResolution 2560x1440\nFullscreen 1\nLaps 9\n";
        let mut taken = Vec::new();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let (k, val) = (it.next().unwrap(), it.next().unwrap());
            if apply(&mut v, k, val, &mut it) {
                taken.push(k);
            }
        }
        assert_eq!(taken, ["GraphicsMode", "RemasterFogStyle", "ViewDistance", "RemasterExposure", "RemasterRevMeter"]);
        assert_eq!((v.mode, v.remaster.fog_style, v.distance, v.remaster.exposure), (RenderMode::Remaster, FogStyle::Volumetric, 2.5, 120));
        assert_eq!(v.remaster.rev_meter.to_array(), [100, 90, 80, 70, 60, 50, 41, 210]);
    }
}
