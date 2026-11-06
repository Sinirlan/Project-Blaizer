//! Gamepad rumble for the player's car (#383) [design choice: the original has no force feedback].
//!
//! `Rumble::mix` is a pure function of the car's state this frame: it returns the strength of the
//! two motors of a pad (strong = low frequency, weak = high frequency), 0..1 each. The surface
//! roughness is the `Bumpy` value of `Data/Material.mat` that the physics already uses
//! (grass 0.3, gravel 0.4, graveltrap 0.7, rock 1.0, sidewalk 0.2) [O]; kerbs have `Bumpy` 0, so
//! they get their own rattle. Controls::set_rumble hands the result to gilrs.

/// Speed (m/s) at which the surface rumble is at full strength (60 km/h).
const FULL_AT: f32 = 16.7;
/// Surface roughness to the low / high motor.
const SURFACE_LOW: f32 = 0.6;
const SURFACE_HIGH: f32 = 0.8;
/// Kerb rattle on the high motor, and its pulse rate (Hz per m/s of speed).
const KERB: f32 = 0.35;
const KERB_HZ_PER_MS: f32 = 0.9;
/// High motor while the rear tyres spin / the brakes lock.
const WHEELSPIN: f32 = 0.3;
const LOCKED: f32 = 0.2;
/// A hit: closing speed (m/s) that gives the full thump, and the decay time of the thump.
const HIT_FULL: f32 = 15.0;
const HIT_DECAY: f32 = 0.25;
/// A landing after more than this many seconds in the air gives a thump of this strength.
const AIR_MIN: f32 = 0.3;
const LANDING: f32 = 0.4;

/// What the mix reads from the car this frame.
#[derive(Clone, Copy, Default)]
pub struct RumbleIn {
    /// speed in m/s
    pub speed: f32,
    /// `Bumpy` of the surface under each wheel, `None` for a wheel in the air
    pub bumpy: [Option<f32>; 4],
    /// the wheel is on a kerb
    pub kerb: [bool; 4],
    pub wheelspin: bool,
    pub brake_locked: bool,
    /// closing speed (m/s) of a hull hit this frame (world, car or prop), 0 for none
    pub hit: f32,
}

#[derive(Default)]
pub struct Rumble {
    /// the thump of a hit or landing, 0..1, decaying
    thump: f32,
    /// phase of the kerb rattle, in cycles
    kerb_phase: f32,
    /// seconds without a wheel on the ground
    air: f32,
}

impl Rumble {
    pub fn reset(&mut self) {
        *self = Rumble::default();
    }

    /// The (strong, weak) motor strengths for this frame, `gain` being the Vibration setting (0..1).
    pub fn mix(&mut self, dt: f32, i: &RumbleIn, gain: f32) -> (f32, f32) {
        let grounded = i.bumpy.iter().flatten().count();
        self.thump = (self.thump - dt / HIT_DECAY).max(0.0);
        if grounded == 0 {
            self.air += dt;
        } else {
            if self.air > AIR_MIN {
                self.thump = self.thump.max(LANDING);
            }
            self.air = 0.0;
        }
        self.thump = self.thump.max((i.hit / HIT_FULL).clamp(0.0, 1.0));
        let (mut low, mut high) = (self.thump, 0.0f32);
        if grounded > 0 {
            let ramp = (i.speed / FULL_AT).clamp(0.0, 1.0);
            let bumpy = i.bumpy.iter().flatten().sum::<f32>() / grounded as f32;
            low = low.max(bumpy * SURFACE_LOW * ramp);
            high = high.max(bumpy * SURFACE_HIGH * ramp);
            if i.kerb.iter().any(|&k| k) {
                self.kerb_phase = (self.kerb_phase + dt * i.speed * KERB_HZ_PER_MS).fract();
                if self.kerb_phase < 0.5 {
                    high = high.max(KERB * ramp);
                }
            }
            if i.wheelspin {
                high = high.max(WHEELSPIN);
            }
            if i.brake_locked && i.speed > 1.0 {
                high = high.max(LOCKED);
            }
        }
        let gain = gain.clamp(0.0, 1.0);
        ((low * gain).clamp(0.0, 1.0), (high * gain).clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(b: f32, speed: f32) -> RumbleIn {
        RumbleIn { speed, bumpy: [Some(b); 4], ..Default::default() }
    }

    #[test]
    fn asphalt_at_speed_is_silent() {
        assert_eq!(Rumble::default().mix(0.02, &on(0.0, 50.0), 1.0), (0.0, 0.0));
    }

    #[test]
    fn standstill_and_zero_gain_are_silent() {
        assert_eq!(Rumble::default().mix(0.02, &on(0.4, 0.0), 1.0), (0.0, 0.0));
        let hit = RumbleIn { hit: 30.0, ..on(0.4, 30.0) };
        assert_eq!(Rumble::default().mix(0.02, &hit, 0.0), (0.0, 0.0));
    }

    #[test]
    fn rougher_surfaces_rumble_harder() {
        let (grass, _) = Rumble::default().mix(0.02, &on(0.3, 30.0), 1.0);
        let (trap, _) = Rumble::default().mix(0.02, &on(0.7, 30.0), 1.0);
        assert!(grass > 0.0 && trap > grass, "{grass} {trap}");
    }

    #[test]
    fn a_hit_thumps_and_decays() {
        let mut r = Rumble::default();
        let (s0, _) = r.mix(0.02, &RumbleIn { hit: 15.0, ..on(0.0, 20.0) }, 1.0);
        assert_eq!(s0, 1.0);
        let mut s = s0;
        for _ in 0..20 {
            s = r.mix(0.02, &on(0.0, 20.0), 1.0).0;
        }
        assert_eq!(s, 0.0);
    }

    #[test]
    fn landing_after_air_time_thumps() {
        let mut r = Rumble::default();
        for _ in 0..20 {
            assert_eq!(r.mix(0.02, &RumbleIn { speed: 30.0, ..Default::default() }, 1.0), (0.0, 0.0));
        }
        assert_eq!(r.mix(0.02, &on(0.0, 30.0), 1.0).0, LANDING);
    }

    #[test]
    fn kerbs_rattle_and_wheelspin_buzzes() {
        let mut r = Rumble::default();
        let kerb = RumbleIn { kerb: [true; 4], ..on(0.0, 30.0) };
        let hi: Vec<f32> = (0..50).map(|_| r.mix(0.02, &kerb, 1.0).1).collect();
        assert!(hi.contains(&KERB) && hi.contains(&0.0));
        let spin = RumbleIn { wheelspin: true, ..on(0.0, 5.0) };
        assert_eq!(Rumble::default().mix(0.02, &spin, 1.0).1, WHEELSPIN);
    }
}
