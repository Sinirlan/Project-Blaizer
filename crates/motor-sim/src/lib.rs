//! Car physics and race rules reimplemented from the model recovered from motor.exe.
//!
//! Every constant and rule cites the original function in `re-notes/03-physics.md` and
//! `re-notes/04-track-car-data.md`; places where this port simplifies or guesses are marked
//! `APPROX`. All vectors use the game's own axes: x right, y up, z forward.

pub mod ai;
pub mod autopilot;
pub mod body;
pub mod car;
pub mod cheats;
pub mod demo;
pub mod demo_stats;
pub mod props;
pub mod race;
pub mod replay;
pub mod session;
pub mod sheet;
pub mod startcam;
pub mod tvcam;
pub mod world;

pub use car::{Car, CarParams, Input, SteerKeys, WorldHit};
pub use session::{AiRow, Difficulty, Driver, Race, Rules};
pub use world::World;
