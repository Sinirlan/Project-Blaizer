//! RIFF WAVE reader for the game's samples (PCM, 8 or 16 bit, mono or stereo; all shipped
//! files are 16-bit mono at 22050 or 16000 Hz). Stereo is mixed down to mono.

use crate::Clip;
use std::sync::Arc;

/// A 16-bit PCM WAV file of interleaved `samples` (`channels` per frame at `rate` Hz).
pub fn encode_pcm16(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let block = u32::from(channels) * 2;
    let mut w = Vec::with_capacity(44 + samples.len() * 2);
    w.extend(b"RIFF");
    w.extend((36 + data_len).to_le_bytes());
    w.extend(b"WAVEfmt ");
    for v in [16u32, 1 | u32::from(channels) << 16, rate, rate * block, block | 16 << 16] {
        w.extend(v.to_le_bytes());
    }
    w.extend(b"data");
    w.extend(data_len.to_le_bytes());
    for s in samples {
        w.extend(s.to_le_bytes());
    }
    w
}

/// Float samples (-1..1) to 16-bit, the way the headless recorders write them.
pub fn to_pcm16(s: f32) -> i16 {
    (s * 32767.0) as i16
}

pub fn decode(d: &[u8]) -> Result<Clip, String> {
    if d.len() < 12 || &d[0..4] != b"RIFF" || &d[8..12] != b"WAVE" {
        return Err("not a RIFF WAVE file".into());
    }
    let u16_ = |o: usize| u16::from_le_bytes([d[o], d[o + 1]]);
    let u32_ = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
    let (mut fmt, mut data) = (None, None);
    let mut p = 12;
    while p + 8 <= d.len() {
        let (id, len) = (&d[p..p + 4], u32_(p + 4) as usize);
        let body = p + 8;
        let end = (body + len).min(d.len());
        match id {
            b"fmt " if len >= 16 => fmt = Some((u16_(body), u16_(body + 2), u32_(body + 4), u16_(body + 14))),
            b"data" => data = Some(&d[body..end]),
            _ => {}
        }
        p = body + len + (len & 1);
    }
    let (format, channels, rate, bits) = fmt.ok_or("no fmt chunk")?;
    let data = data.ok_or("no data chunk")?;
    if format != 1 {
        return Err(format!("unsupported WAVE format {format}"));
    }
    let ch = channels.max(1) as usize;
    let samples: Vec<f32> = match bits {
        16 => data.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect(),
        8 => data.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(),
        _ => return Err(format!("unsupported {bits}-bit samples")),
    };
    let mono = samples.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect();
    Ok(Clip { rate, data: Arc::new(mono), loop_points: None })
}

#[cfg(test)]
mod tests {
    #[test]
    fn mono16() {
        let mut d = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0".to_vec();
        d.extend(1u16.to_le_bytes());
        d.extend(1u16.to_le_bytes());
        d.extend(22050u32.to_le_bytes());
        d.extend(44100u32.to_le_bytes());
        d.extend(2u16.to_le_bytes());
        d.extend(16u16.to_le_bytes());
        d.extend(b"data\x04\0\0\0");
        d.extend(16384i16.to_le_bytes());
        d.extend((-32768i16).to_le_bytes());
        let c = super::decode(&d).unwrap();
        assert_eq!(c.rate, 22050);
        assert_eq!(*c.data, vec![0.5, -1.0]);
    }
}
