//! cpal output: the device callback renders the shared mixer. Also the list of outputs the player can pick on
//! the Sound screen (#260) and opening one by its saved id.

use crate::Mixer;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use std::sync::{Arc, Mutex};

pub type Stream = cpal::Stream;

/// An audio output the player can choose: `id` is the host's stable id (what settings.cfg keeps), `name` is
/// what the menu shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputDevice {
    pub id: String,
    pub name: String,
}

/// The ALSA host lists every PCM plugin (`hw`, `plughw`, rate converters, `null`, `surround51`, ...), 50 or
/// so entries on a desktop. The ones that are an output of their own: a card's default (`sysdefault`, its
/// analog out), each HDMI / DisplayPort port (`hdmi`) and a digital out (`iec958`). The system default (the
/// sound server's) is the Sound screen's first choice already.
fn keep_alsa(pcm: &str) -> bool {
    matches!(pcm.split(':').next(), Some("sysdefault" | "hdmi" | "iec958"))
}

/// The outputs of the default host worth offering, in the host's order, without the system default.
pub fn output_devices() -> Vec<OutputDevice> {
    let host = cpal::default_host();
    let mut out: Vec<OutputDevice> = Vec::new();
    for d in host.output_devices().into_iter().flatten() {
        let Ok(id) = d.id() else { continue };
        let id = id.to_string();
        if let Some(pcm) = id.strip_prefix("alsa:") {
            if !keep_alsa(pcm) {
                continue;
            }
        }
        let name = d.description().map_or_else(|_| d.to_string(), |x| x.name().to_string());
        if !out.iter().any(|o| o.id == id) {
            out.push(OutputDevice { id, name });
        }
    }
    out
}

/// The name of the system's default output (what "System default" plays on), if there is one and it says more
/// than "Default Audio Device" (the ALSA `default` PCM, which hides the sound server behind it).
pub fn default_output_name() -> Option<String> {
    let d = cpal::default_host().default_output_device()?;
    if d.id().is_ok_and(|id| id.to_string() == "alsa:default") {
        return None;
    }
    Some(d.description().map_or_else(|_| d.to_string(), |x| x.name().to_string()))
}

/// Start the mixer on the output with this saved `id`, or on the system default (`None`). The mixer's rate
/// follows the device. `Err` = why that output could not be used.
pub fn start(device: Option<&str>, mixer: &Arc<Mutex<Mixer>>) -> Result<Stream, String> {
    let host = cpal::default_host();
    let device = match device {
        // by the same string `output_devices` listed (cpal's `device_by_id` rewrites ALSA ids that have no `,DEV=`, such as
        // `sysdefault:CARD=PCH`, and then finds nothing)
        Some(id) => host.output_devices().map_err(|e| e.to_string())?.find(|d| d.id().is_ok_and(|i| i.to_string() == id)).ok_or("not connected")?,
        None => host.default_output_device().ok_or("no output device")?,
    };
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let format = config.sample_format();
    let config: StreamConfig = config.into();
    if let Ok(mut m) = mixer.lock() {
        m.out_rate = config.sample_rate;
    }
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, config, mixer.clone()),
        SampleFormat::I16 => build::<i16>(&device, config, mixer.clone()),
        SampleFormat::U16 => build::<u16>(&device, config, mixer.clone()),
        SampleFormat::I32 => build::<i32>(&device, config, mixer.clone()),
        f => Err(format!("unsupported sample format {f}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn build<T: SizedSample + FromSample<f32>>(device: &cpal::Device, config: StreamConfig, mixer: Arc<Mutex<Mixer>>) -> Result<Stream, String> {
    let channels = config.channels as usize;
    let mut buf: Vec<f32> = Vec::new();
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
                buf.resize(out.len(), 0.0);
                match mixer.lock() {
                    Ok(mut m) => m.render(&mut buf, channels),
                    Err(_) => buf.iter_mut().for_each(|s| *s = 0.0),
                }
                for (o, s) in out.iter_mut().zip(&buf) {
                    *o = T::from_sample(*s);
                }
            },
            |e| eprintln!("audio stream: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::keep_alsa;

    #[test]
    fn alsa_list_keeps_one_entry_per_output() {
        for pcm in ["sysdefault:CARD=PCH", "hdmi:CARD=HDMI,DEV=0", "iec958:CARD=Generic,DEV=0"] {
            assert!(keep_alsa(pcm), "{pcm}");
        }
        for pcm in ["null", "default", "pipewire", "pulse", "hw:CARD=0,DEV=3", "plughw:CARD=1,DEV=0", "front:CARD=V2,DEV=0", "surround51:CARD=V2,DEV=0", "usbstream:CARD=HDMI", "lavrate"] {
            assert!(!keep_alsa(pcm), "{pcm}");
        }
    }
}
