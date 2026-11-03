//! The original minimap's projection (CLASSIC 0x412c0c / 0x412e1c): the race HUD's map, the Track screen's map
//! and the pause menu's map place cars and laps with it, so every map in the game agrees. (The REMASTER
//! generated minimap images this module once wrote, `motorview --minimap`, are gone: the Race line HUD draws the
//! lap as vector strokes, #323 phase 6.)

use glam::Vec3;

/// CLASSIC: world (game space) -> minimap pixel, 0x412e1c..0x412ebb with TrackMapInfo
/// (x0, y_cam, z0, angle): s = 1/(y_cam - y); X,Z = (x - x0, z - z0) * 256 s;
/// a = angle + 90 deg; map = 0.5 * (sin a X + cos a Z + 64, cos a X - sin a Z + 80).
/// A pinhole camera at (x0, y_cam, z0) looking down, so straight edges stay straight.
pub fn map_point(info: [f32; 4], p: Vec3) -> (f32, f32) {
    let s = 1.0 / (info[1] - p.y).max(1.0);
    let (x, z) = ((p.x - info[0]) * 256.0 * s, (p.z - info[2]) * 256.0 * s);
    let a = (info[3] + 90.0).to_radians();
    (0.5 * (a.sin() * x + a.cos() * z + 64.0), 0.5 * (a.cos() * x - a.sin() * z + 80.0))
}

/// CLASSIC 0x411fbd..0x4121ec: the map name is compared (case-insensitively) with
/// "tracks\track5\data\volvo3" -> markers shifted (-10, 0), and
/// "tracks\track3\data\okkcity" -> (-12, -2); every other map (0, 0).
pub fn map_offset(name: &str) -> (f32, f32) {
    match name.replace('/', "\\").to_ascii_lowercase().as_str() {
        "tracks\\track5\\data\\volvo3" => (-10.0, 0.0),
        "tracks\\track3\\data\\okkcity" => (-12.0, -2.0),
        _ => (0.0, 0.0),
    }
}

/// The track's TrackMapInfo, as the game reads it.
pub fn map_info(trk: &motor_data::cfg::Config) -> [f32; 4] {
    trk.get("TrackMapInfo").map(|e| [e.f32(0), e.f32(1), e.f32(2), e.f32(3)].map(|v| v.unwrap_or(0.0))).unwrap_or([0.0; 4])
}
