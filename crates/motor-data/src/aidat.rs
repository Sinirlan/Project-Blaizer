//! AI racing line (Tracks/trackN/AI/ai.dat). Loader 0x408d74.

use crate::bytes::{err, f32_, i32_, u32_, Result};

#[derive(Debug, Clone)]
pub struct Node {
    pub x: f32,
    pub z: f32,
    pub w_left: f32,
    pub w_right: f32,
    pub dir: [f32; 3],
    /// signed turn radius at this node in metres (0 = straight); the AI's target speed is
    /// sqrt(|radius| · 9.82 · AI[8]) (0x407800)
    pub radius: f32,
    pub corner: i32,
}

#[derive(Debug, Clone)]
pub struct Corner {
    pub id: i32,
    pub nodes: i32,
    /// signed minimum turn radius of the corner in metres (0 = straight section)
    pub min_radius: f32,
}

#[derive(Debug, Clone)]
pub struct RacingLine {
    pub nodes: Vec<Node>,
    pub corners: Vec<Corner>,
}

pub fn parse(d: &[u8]) -> Result<RacingLine> {
    let n = u32_(d, 0)? as usize;
    let nodes = (0..n)
        .map(|i| {
            let o = 4 + 36 * i;
            Ok(Node {
                x: f32_(d, o)?,
                z: f32_(d, o + 4)?,
                w_left: f32_(d, o + 8)?,
                w_right: f32_(d, o + 12)?,
                dir: [f32_(d, o + 16)?, f32_(d, o + 20)?, f32_(d, o + 24)?],
                radius: f32_(d, o + 28)?,
                corner: i32_(d, o + 32)?,
            })
        })
        .collect::<Result<_>>()?;
    let p = 4 + 36 * n;
    let m = u32_(d, p)? as usize;
    if p + 4 + 12 * m != d.len() {
        return err("ai.dat size mismatch");
    }
    let corners = (0..m)
        .map(|i| {
            let o = p + 4 + 12 * i;
            Ok(Corner { id: i32_(d, o)?, nodes: i32_(d, o + 4)?, min_radius: f32_(d, o + 8)? })
        })
        .collect::<Result<_>>()?;
    Ok(RacingLine { nodes, corners })
}
