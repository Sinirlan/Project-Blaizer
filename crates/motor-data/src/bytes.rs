//! Bounds-checked little-endian readers.

use std::fmt;

#[derive(Debug, Clone)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error(msg.into()))
}

pub fn slice(d: &[u8], o: usize, n: usize) -> Result<&[u8]> {
    d.get(o..o.checked_add(n).ok_or_else(|| Error("offset overflow".into()))?)
        .ok_or_else(|| Error(format!("read of {n} bytes at {o:#x} past end ({:#x})", d.len())))
}

pub fn u8_(d: &[u8], o: usize) -> Result<u8> {
    Ok(slice(d, o, 1)?[0])
}
pub fn u16_(d: &[u8], o: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(slice(d, o, 2)?.try_into().unwrap()))
}
pub fn i16_(d: &[u8], o: usize) -> Result<i16> {
    Ok(i16::from_le_bytes(slice(d, o, 2)?.try_into().unwrap()))
}
pub fn u32_(d: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(slice(d, o, 4)?.try_into().unwrap()))
}
pub fn i32_(d: &[u8], o: usize) -> Result<i32> {
    Ok(i32::from_le_bytes(slice(d, o, 4)?.try_into().unwrap()))
}
pub fn f32_(d: &[u8], o: usize) -> Result<f32> {
    Ok(f32::from_le_bytes(slice(d, o, 4)?.try_into().unwrap()))
}
pub fn f64_(d: &[u8], o: usize) -> Result<f64> {
    Ok(f64::from_le_bytes(slice(d, o, 8)?.try_into().unwrap()))
}

pub fn vec3(d: &[u8], o: usize) -> Result<[f32; 3]> {
    Ok([f32_(d, o)?, f32_(d, o + 4)?, f32_(d, o + 8)?])
}
pub fn vec4(d: &[u8], o: usize) -> Result<[f32; 4]> {
    Ok([f32_(d, o)?, f32_(d, o + 4)?, f32_(d, o + 8)?, f32_(d, o + 12)?])
}

/// Latin-1 string up to the first NUL (or `max` bytes).
pub fn cstr(d: &[u8], o: usize, max: Option<usize>) -> Result<String> {
    let tail = d.get(o..).ok_or_else(|| Error(format!("string at {o:#x} past end")))?;
    let tail = match max {
        Some(n) => &tail[..n.min(tail.len())],
        None => tail,
    };
    let end = tail.iter().position(|&b| b == 0);
    if max.is_none() && end.is_none() {
        return err(format!("unterminated string at {o:#x}"));
    }
    Ok(latin1(&tail[..end.unwrap_or(tail.len())]))
}

pub fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}
