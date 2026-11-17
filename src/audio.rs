//! Recording from this machine's default microphone, as a stream of bytes to send elsewhere
//! (see `remote::mic`, which feeds it to an ssh host's default ALSA mic).
//!
//! The wire format is fixed, whatever the device records: signed 16-bit little-endian, mono,
//! [`RATE`] Hz. That's what speech recognizers want, it's small (32 KB/s), and the remote side
//! converts it to whatever an app there asks for.
//!
//! `cpal` does the per-OS work (CoreAudio on macOS, ALSA on Linux, WASAPI on Windows), so this
//! file has no platform code. The stream lives on a thread of its own, since a `cpal::Stream`
//! isn't `Send` on every backend; dropping the [`Capture`] stops it.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, SampleFormat, SizedSample};

/// Samples per second sent.
pub const RATE: u32 = 16_000;

/// Why recording couldn't start.
#[derive(Debug, PartialEq, Eq)]
pub enum CaptureError {
    /// There's no input device.
    NoMic,
    /// The OS refused (macOS: System Settings → Privacy & Security → Microphone).
    PermissionDenied,
    Other(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoMic => f.write_str("no microphone found"),
            Self::PermissionDenied => f.write_str("microphone access was denied"),
            Self::Other(e) => f.write_str(e),
        }
    }
}

impl From<cpal::Error> for CaptureError {
    fn from(e: cpal::Error) -> Self {
        match e.kind() {
            ErrorKind::PermissionDenied => Self::PermissionDenied,
            ErrorKind::DeviceNotAvailable => Self::NoMic,
            _ => Self::Other(e.to_string()),
        }
    }
}

/// The microphone, recording. Dropping it stops recording.
pub struct Capture {
    /// Dropped to tell the stream's thread to stop.
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
    failed: Arc<AtomicBool>,
}

impl Capture {
    /// Start recording from the default input device, sending chunks of audio in the wire
    /// format to `sink` until dropped. Returns once the device is open (or failed to open).
    pub fn start(sink: Sender<Vec<u8>>) -> Result<Self, CaptureError> {
        let failed = Arc::new(AtomicBool::new(false));
        let (stop, stopped) = mpsc::channel::<()>();
        let (opened, open_result) = mpsc::channel();
        let flag = failed.clone();
        let thread = std::thread::Builder::new()
            .name("nex-mic".into())
            .spawn(move || match open(sink, flag) {
                Ok(stream) => {
                    let _ = opened.send(Ok(()));
                    // Wait until the `Capture` is dropped; the stream stops with this thread.
                    let _ = stopped.recv();
                    drop(stream);
                }
                Err(e) => {
                    let _ = opened.send(Err(e));
                }
            })
            .map_err(|e| CaptureError::Other(e.to_string()))?;
        let capture = Self { stop: Some(stop), thread: Some(thread), failed };
        match open_result.recv() {
            Ok(Ok(())) => Ok(capture),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(CaptureError::Other("the recording thread stopped".into())),
        }
    }

    /// Whether the device failed after it opened (unplugged, say): no more audio is coming.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Open the default mic and start it, sending wire-format chunks to `sink`.
fn open(sink: Sender<Vec<u8>>, failed: Arc<AtomicBool>) -> Result<cpal::Stream, CaptureError> {
    let device = cpal::default_host().default_input_device().ok_or(CaptureError::NoMic)?;
    let supported = device.default_input_config()?;
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, config, sink, failed)?,
        SampleFormat::F64 => build::<f64>(&device, config, sink, failed)?,
        SampleFormat::I16 => build::<i16>(&device, config, sink, failed)?,
        SampleFormat::I32 => build::<i32>(&device, config, sink, failed)?,
        SampleFormat::I8 => build::<i8>(&device, config, sink, failed)?,
        SampleFormat::U16 => build::<u16>(&device, config, sink, failed)?,
        SampleFormat::U8 => build::<u8>(&device, config, sink, failed)?,
        other => return Err(CaptureError::Other(format!("the microphone records {other:?} samples, which nex can't read"))),
    };
    stream.play()?;
    Ok(stream)
}

fn build<T>(device: &cpal::Device, config: cpal::StreamConfig, sink: Sender<Vec<u8>>, failed: Arc<AtomicBool>) -> Result<cpal::Stream, CaptureError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels).max(1);
    let mut resampler = Resampler::new(config.sample_rate, RATE);
    let mut mono = Vec::new();
    let mut out = Vec::new();
    let data = move |samples: &[T], _: &cpal::InputCallbackInfo| {
        mono.clear();
        mono.extend(samples.chunks(channels).map(|frame| frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / frame.len() as f32));
        out.clear();
        resampler.push(&mono, &mut out);
        if !out.is_empty() {
            let _ = sink.send(out.iter().flat_map(|s| s.to_le_bytes()).collect());
        }
    };
    let error = move |e: cpal::Error| {
        // The default device changing is followed by the stream; anything else ends it.
        if e.kind() != ErrorKind::DeviceChanged {
            eprintln!("nex: microphone: {e}");
            failed.store(true, Ordering::Relaxed);
        }
    };
    Ok(device.build_input_stream::<T, _, _>(config, data, error, None)?)
}

/// Converts a mono stream from one sample rate to another, in chunks, by linear interpolation.
/// Going down (48 kHz to 16 kHz, say), a moving average first takes out what the lower rate
/// can't hold, so it doesn't fold back as noise.
struct Resampler {
    /// Input samples per output sample.
    step: f64,
    /// Where the next output sample falls, counted from `last` (0) into the next chunk.
    pos: f64,
    /// The previous chunk's last (smoothed) sample.
    last: f32,
    /// The moving average's recent inputs, and their sum.
    window: VecDeque<f32>,
    width: usize,
    sum: f32,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        let step = f64::from(from.max(1)) / f64::from(to.max(1));
        let width = if step > 1.0 { step.round() as usize } else { 1 };
        Self { step, pos: 0.0, last: 0.0, window: VecDeque::with_capacity(width + 1), width, sum: 0.0 }
    }

    fn smooth(&mut self, x: f32) -> f32 {
        if self.width <= 1 {
            return x;
        }
        self.window.push_back(x);
        self.sum += x;
        if self.window.len() > self.width {
            self.sum -= self.window.pop_front().unwrap_or(0.0);
        }
        self.sum / self.window.len() as f32
    }

    /// Resample `input`, appending 16-bit samples to `out`.
    fn push(&mut self, input: &[f32], out: &mut Vec<i16>) {
        let input: Vec<f32> = input.iter().map(|&x| self.smooth(x)).collect();
        // Index 0 is `last`, index k is input[k - 1]; interpolating between i and i + 1 needs
        // i + 1 <= len.
        let len = input.len() as f64;
        while self.pos < len {
            let i = self.pos as usize;
            let frac = (self.pos - i as f64) as f32;
            let a = if i == 0 { self.last } else { input[i - 1] };
            let b = input[i];
            out.push(to_i16(a + (b - a) * frac));
            self.pos += self.step;
        }
        self.pos -= len;
        if let Some(&l) = input.last() {
            self.last = l;
        }
    }
}

fn to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(from: u32, input: &[f32], chunk: usize) -> Vec<i16> {
        let mut r = Resampler::new(from, RATE);
        let mut out = Vec::new();
        for c in input.chunks(chunk) {
            r.push(c, &mut out);
        }
        out
    }

    #[test]
    fn same_rate_passes_through_one_sample_late() {
        let input: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
        let out = run(RATE, &input, 7);
        assert_eq!(out.len(), input.len());
        assert_eq!(out[0], 0);
        for (o, i) in out[1..].iter().zip(&input) {
            assert_eq!(*o, to_i16(*i));
        }
    }

    #[test]
    fn downsampling_keeps_duration_across_chunks() {
        let input = vec![0.25f32; 48_000];
        for chunk in [480, 512, 1000, 48_000] {
            let out = run(48_000, &input, chunk);
            assert!((out.len() as i64 - 16_000).abs() <= 1, "chunk {chunk}: {}", out.len());
        }
        let out = run(44_100, &vec![0.0f32; 44_100], 441);
        assert!((out.len() as i64 - 16_000).abs() <= 1, "{}", out.len());
    }

    #[test]
    fn downsampling_keeps_a_steady_level_and_drops_the_highest_tone() {
        let out = run(48_000, &vec![0.5f32; 4800], 480);
        assert!(out[10..].iter().all(|&s| (s - to_i16(0.5)).abs() <= 1));
        // A tone at 24 kHz (alternating samples) can't exist at 16 kHz: it averages away
        // instead of folding back into the audible range.
        let tone: Vec<f32> = (0..4800).map(|i| if i % 2 == 0 { 0.8 } else { -0.8 }).collect();
        let out = run(48_000, &tone, 480);
        let peak = out[10..].iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(peak < to_i16(0.8) as u16 / 2, "{peak}");
    }

    #[test]
    fn upsampling_interpolates() {
        let out = run(8_000, &[0.0, 1.0, 0.0], 3);
        assert_eq!(out.len(), 6);
        assert_eq!(out[2], to_i16(0.0));
        assert_eq!(out[3], to_i16(0.5));
        assert_eq!(out[4], to_i16(1.0));
    }

    #[test]
    fn clamps() {
        assert_eq!(to_i16(2.0), i16::MAX);
        assert_eq!(to_i16(-2.0), -i16::MAX);
    }
}
