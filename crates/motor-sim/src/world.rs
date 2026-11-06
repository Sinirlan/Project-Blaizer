//! Static world for the simulation: track collision mesh + surface materials.

use glam::Vec3;
use motor_data::cfg::Config;
use motor_data::col::{self, Collision};
use motor_data::fs;
use std::path::Path;

/// One `Data/Material.mat` entry (parser 0x47c0f8; defaults are the code's own).
#[derive(Debug, Clone)]
pub struct Material {
    pub name: String,
    pub spin_speed_limit: f32,
    pub spin_rot_limit: f32,
    pub x_spin: f32,
    pub x_nospin: f32,
    pub z_spin: f32,
    pub z_nospin: f32,
    pub rot_spin: f32,
    pub rot_nospin: f32,
    pub spin_turn_reduce: f32,
    pub nospin_turn_reduce: f32,
    pub bumpy: f32,
    /// SpinWheelLimit (+0x4c) / NoSpinWheelLimit (+0x50): the wheelspin threshold of the surface
    /// while the rear tyres spin / grip (03-physics 3.2, P[0x35])
    pub spin_wheel_limit: f32,
    pub nospin_wheel_limit: f32,
    /// ContactAnimID (+0x370): the tyre effect on this surface (1 smoke, 2 grass, 3 dirt,
    /// 4 steel, only with a cheat flag); 0 none. Read by 0x47de93.
    pub contact_anim: u32,
    /// SparkAnimID (+0x36c): 1 = hull contacts throw sparks (0x481401)
    pub spark_anim: u32,
    /// SkidMark (+0x378): the kind of tyre mark the surface takes (1 asphalt, 2 concrete, 3 grass,
    /// 4 gravel); 0 none. The mark code only handles 1..=4 (jump table 0x446634, read at 0x446705) [O].
    pub skid_mark: u32,
    /// SlideType (+0x374, loader 0x47ca85): 1 = a slide plays `slide1.wav` and ends with the
    /// `slidee1/2` tail instead of the SndSlide loop (asphalt only; 07-audio 7.2)
    pub slide_type: u32,
    /// Snd* lines: kind (lower-case key without "snd", e.g. "slide", "hardimpulse") ->
    /// sample, volume, pitch. Format: `SndX sample loop priority volume pitch loopstart loopend`.
    pub sounds: Vec<(String, MatSound)>,
}

#[derive(Debug, Clone)]
pub struct MatSound {
    pub sample: String,
    pub looping: bool,
    /// the record's priority (+0x10): SndContact's picks the sound surface among the wheels (0x484126)
    pub priority: f32,
    pub volume: f32,
    pub pitch: f32,
    /// loop region in samples, when given (both 0 = whole sample; an end at or before the start,
    /// as SndSpin's `36102 0`, runs to the sample's end [I])
    pub loop_points: Option<(usize, usize)>,
}

impl Material {
    pub fn sound(&self, kind: &str) -> Option<&MatSound> {
        self.sounds.iter().find(|(k, _)| k == kind).map(|(_, s)| s)
    }
}

impl Default for Material {
    fn default() -> Self {
        Material {
            name: "default".into(),
            spin_speed_limit: 7.0,
            spin_rot_limit: 26000.0,
            x_spin: 0.2,
            x_nospin: 0.000555,
            z_spin: 0.796714,
            z_nospin: 0.858086,
            rot_spin: 0.010429,
            rot_nospin: 7e-6,
            spin_turn_reduce: 0.9,
            nospin_turn_reduce: 1.0,
            bumpy: 0.0,
            spin_wheel_limit: 2000.0,
            nospin_wheel_limit: 3000.0,
            contact_anim: 0,
            spark_anim: 0,
            skid_mark: 0,
            slide_type: 0,
            sounds: Vec::new(),
        }
    }
}

pub fn parse_materials(c: &Config) -> Vec<Material> {
    let mut out: Vec<Material> = Vec::new();
    for e in &c.entries {
        let k = e.key.to_ascii_lowercase();
        if k == "materialname" {
            out.push(Material { name: e.values.first().cloned().unwrap_or_default(), ..Default::default() });
            continue;
        }
        if let (Some(kind), Some(m)) = (k.strip_prefix("snd"), out.last_mut()) {
            let f = |i: usize| e.f32(i).unwrap_or(0.0);
            let (ls, le) = (f(5) as usize, f(6) as usize);
            m.sounds.push((
                kind.to_string(),
                MatSound {
                    sample: e.values.first().cloned().unwrap_or_default(),
                    looping: f(1) != 0.0,
                    priority: f(2),
                    volume: f(3),
                    pitch: f(4),
                    loop_points: match (ls, le) {
                        (0, 0) => None,
                        (s, e) if e > s => Some((s, e)),
                        (s, _) => Some((s, usize::MAX)),
                    },
                },
            ));
            continue;
        }
        let (Some(m), Some(v)) = (out.last_mut(), e.f32(0)) else { continue };
        match k.as_str() {
            "spinspeedlimit" => m.spin_speed_limit = v,
            "spinrotlimit" => m.spin_rot_limit = v,
            "xspinfriction" => m.x_spin = v,
            "xnospinfriction" => m.x_nospin = v,
            "zspinfriction" => m.z_spin = v,
            "znospinfriction" => m.z_nospin = v,
            "rotspinfriction" => m.rot_spin = v,
            "rotnospinfriction" => m.rot_nospin = v,
            "spinturnreduce" => m.spin_turn_reduce = v,
            "nospinturnreduce" => m.nospin_turn_reduce = v,
            "bumpy" => m.bumpy = v,
            "spinwheellimit" => m.spin_wheel_limit = v,
            "nospinwheellimit" => m.nospin_wheel_limit = v,
            "contactanimid" => m.contact_anim = v as u32,
            "sparkanimid" => m.spark_anim = v as u32,
            "skidmark" => m.skid_mark = v as u32,
            "slidetype" => m.slide_type = v as u32,
            _ => {}
        }
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub struct RayHit {
    /// fraction along the query segment
    pub t: f32,
    pub point: Vec3,
    pub normal: Vec3,
    /// the face's plane constant (`normal . p + d = 0`), kept for `World::plane_hit`
    pub plane_d: f32,
    /// index into `World::materials`
    pub material: usize,
}

pub struct World {
    pub col: Collision,
    pub materials: Vec<Material>,
    /// COL face material index (.SRF order) -> index into `materials`
    surface: Vec<usize>,
    /// `GRAVITY` normally, half with the low-gravity flag (0x4814ac)
    pub gravity: f32,
}

motor_tune::tunable! {
    /// The game's gravity (0x4814ac) [O]. A race takes it when the track loads.
    /// range: 1..40
    /// unit: m/s²
    /// applies: restart
    pub GRAVITY: f32 = 15.81;
}

impl World {
    pub fn load(root: &Path, track: u32) -> Result<World, String> {
        let trk = fs::config(root, &format!("Game/Track{track}.trk"))?;
        let get = |k: &str| trk.get(k).and_then(|e| e.values.first().cloned()).ok_or(format!("Track{track}.trk: no {k}"));
        let col = col::parse(&fs::read(root, &get("CollisionName")?)?).map_err(|e| e.to_string())?;
        let mut materials = parse_materials(&fs::config(root, "Data/Material.mat")?);
        let srf = fs::config(root, &get("MaterialsName")?)?;
        let mut surface = Vec::new();
        for e in srf.all("Material") {
            let name = e.values.first().cloned().unwrap_or_default();
            let idx = materials.iter().position(|m| m.name.eq_ignore_ascii_case(&name)).unwrap_or_else(|| {
                materials.push(Material { name: name.clone(), ..Default::default() });
                materials.len() - 1
            });
            surface.push(idx);
        }
        if materials.is_empty() {
            materials.push(Material::default());
        }
        Ok(World { col, materials, surface, gravity: GRAVITY.get() })
    }

    pub fn raycast(&self, a: Vec3, b: Vec3) -> Option<RayHit> {
        let h = self.col.raycast(a.to_array(), b.to_array())?;
        Some(RayHit {
            t: h.t,
            point: Vec3::from(h.point),
            normal: Vec3::from(h.normal),
            plane_d: h.d,
            material: self.surface.get(h.material as usize).copied().unwrap_or(0),
        })
    }

    /// Re-test a segment against the plane of an earlier hit (0x416480, see `Collision::plane_hit`).
    pub fn plane_hit(normal: Vec3, d: f32, a: Vec3, b: Vec3) -> Option<f32> {
        Collision::plane_hit(normal.to_array(), d, a.to_array(), b.to_array())
    }

    /// `materials` index of a COL face material (.SRF order).
    pub fn material_of(&self, face_material: u16) -> usize {
        self.surface.get(face_material as usize).copied().unwrap_or(0)
    }

    /// Ground height under (x, z), searching from `y_top` downwards.
    pub fn ground(&self, x: f32, z: f32, y_top: f32) -> Option<f32> {
        self.raycast(Vec3::new(x, y_top, z), Vec3::new(x, y_top - 200.0, z)).map(|h| h.point.y)
    }
}
