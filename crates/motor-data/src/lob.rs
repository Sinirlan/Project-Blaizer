//! LOB "line objects": vector wireframes (menu car/track previews). Plain or TBL-wrapped.

use crate::bytes::{err, u32_, vec3, Result};

#[derive(Debug, Clone)]
pub struct LineObject {
    pub vertices: Vec<[f32; 3]>,
    /// 20 trailing bytes per vertex (zero in car files; meaning unknown)
    pub extra: Vec<[u8; 20]>,
    pub edges: Vec<[u32; 2]>,
}

pub fn parse(d: &[u8]) -> Result<LineObject> {
    let (nv, ne) = (u32_(d, 0)? as usize, u32_(d, 4)? as usize);
    if nv.checked_mul(32).and_then(|v| v.checked_add(8 + ne.checked_mul(8)?)) != Some(d.len()) {
        return err("size mismatch: not a LOB");
    }
    let vertices = (0..nv).map(|i| vec3(d, 8 + 32 * i)).collect::<Result<_>>()?;
    let extra = (0..nv).map(|i| d[8 + 32 * i + 12..8 + 32 * i + 32].try_into().unwrap()).collect();
    let eo = 8 + 32 * nv;
    let edges = (0..ne)
        .map(|i| Ok([u32_(d, eo + 8 * i)?, u32_(d, eo + 8 * i + 4)?]))
        .collect::<Result<_>>()?;
    Ok(LineObject { vertices, extra, edges })
}

#[cfg(test)]
mod tests {
    #[test]
    fn tiny_wireframe() {
        let mut d = Vec::new();
        d.extend_from_slice(&2u32.to_le_bytes());
        d.extend_from_slice(&1u32.to_le_bytes());
        for x in [1.0f32, 2.0] {
            d.extend_from_slice(&x.to_le_bytes());
            d.extend_from_slice(&[0; 28]);
        }
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(&1u32.to_le_bytes());
        let l = super::parse(&d).unwrap();
        assert_eq!(l.vertices[1][0], 2.0);
        assert_eq!(l.edges, vec![[0, 1]]);
        assert!(super::parse(&d[..d.len() - 1]).is_err());
    }
}
