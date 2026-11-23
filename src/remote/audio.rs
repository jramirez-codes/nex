//! Recording from this machine's default microphone, as a stream of bytes to send elsewhere
//! (see `remote::mic`, which feeds it to an ssh host's default ALSA mic).
//!
//! The wire format is fixed, whatever the device records: signed 16-bit little-endian, mono,
//! [`RATE`] Hz. That's what speech recognizers want, it's small (32 KB/s), and the remote side
//! converts it to whatever an app there asks for.
//!
//! `cpal` does the per-OS work (CoreAudio on macOS and iOS, ALSA on Linux, WASAPI on Windows),
//! so this file has no platform code beyond `platform::prepare_mic` (iOS's audio session). The stream lives on a thread of its own, since a `cpal::Stream`
//! isn't `Send` on every backend; dropping the [`Capture`] stops it.

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
///
/// It asks the device for the wire format's rate and one channel first: on Linux the sound
/// server (PipeWire, PulseAudio) then converts, as it does for any program recording there.
/// Devices that only record at their own rate (CoreAudio's, usually) get [`Resampler`].
fn open(sink: Sender<Vec<u8>>, failed: Arc<AtomicBool>) -> Result<cpal::Stream, CaptureError> {
    // iOS: switch the audio session to recording first, or there is no input to open.
    crate::platform::prepare_mic().map_err(|e| match e {
        crate::platform::MicRefused::Denied => CaptureError::PermissionDenied,
        crate::platform::MicRefused::Failed(msg) => CaptureError::Other(msg),
    })?;
    let device = cpal::default_host().default_input_device().ok_or(CaptureError::NoMic)?;
    let supported = wire_config(&device).map_or_else(|| device.default_input_config(), Ok)?;
    let mut config = supported.config();
    // Small, steady chunks: each one waits on the next to fill, and a recording's start (and
    // the cushion against jitter it has to cover) waits on the first.
    if let cpal::SupportedBufferSize::Range { min, max } = *supported.buffer_size() {
        let frames = config.sample_rate / 50;
        if (min..=max).contains(&frames) {
            config.buffer_size = cpal::BufferSize::Fixed(frames);
        }
    }
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

/// The device's config for [`RATE`], one channel, in floats or 16-bit samples, if it has one.
fn wire_config(device: &cpal::Device) -> Option<cpal::SupportedStreamConfig> {
    let ranges: Vec<_> = device.supported_input_configs().ok()?.collect();
    [SampleFormat::F32, SampleFormat::I16].into_iter().find_map(|format| {
        ranges
            .iter()
            .find(|r| r.channels() == 1 && r.sample_format() == format && r.min_sample_rate() <= RATE && RATE <= r.max_sample_rate())
            .map(|r| r.with_sample_rate(RATE))
    })
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

/// Output samples on each side of a windowed-sinc kernel: its length, so how sharp the
/// filter's edge is (about 6% of the output rate wide).
const HALF_TAPS: f64 = 24.0;
/// Kernel table entries per input sample, interpolated between.
const PHASES: usize = 256;

/// Converts a mono stream from one sample rate to another, in chunks: band-limited
/// interpolation with a Blackman-windowed sinc. Its cutoff, 90% of the lower rate's Nyquist
/// frequency (7.2 kHz for 16 kHz), keeps the consonants speech recognizers listen for, while
/// anything the output rate can't hold is filtered out rather than folded back as noise.
struct Resampler {
    /// Input samples per output sample; 1 passes samples straight through.
    step: f64,
    /// Kernel half-width, in input samples.
    half: usize,
    /// The kernel at `PHASES` points per input sample, from 0 to `half`.
    table: Vec<f32>,
    /// Input not yet used up, and the stream position of its first sample.
    buf: Vec<f32>,
    base: u64,
    /// The stream position (in input samples) of the next output sample.
    next: f64,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        let step = f64::from(from.max(1)) / f64::from(to.max(1));
        // Cutoff in cycles per input sample.
        let cutoff = 0.45 / step.max(1.0);
        let half = (HALF_TAPS * step.max(1.0)).ceil() as usize;
        let table = (0..=half * PHASES)
            .map(|i| {
                let x = i as f64 / PHASES as f64;
                let sinc = if x == 0.0 { 1.0 } else { (std::f64::consts::PI * 2.0 * cutoff * x).sin() / (std::f64::consts::PI * 2.0 * cutoff * x) };
                let w = x / half as f64;
                let window = 0.42 + 0.5 * (std::f64::consts::PI * w).cos() + 0.08 * (2.0 * std::f64::consts::PI * w).cos();
                (2.0 * cutoff * sinc * window) as f32
            })
            .collect();
        Self { step, half, table, buf: Vec::new(), base: 0, next: 0.0 }
    }

    /// The kernel at `x` input samples from its center.
    fn kernel(&self, x: f64) -> f32 {
        let at = x.abs() * PHASES as f64;
        let i = at as usize;
        let (Some(&a), b) = (self.table.get(i), self.table.get(i + 1).copied().unwrap_or(0.0)) else {
            return 0.0;
        };
        a + (b - a) * (at - i as f64) as f32
    }

    /// Resample `input`, appending 16-bit samples to `out`.
    fn push(&mut self, input: &[f32], out: &mut Vec<i16>) {
        if self.step == 1.0 {
            out.extend(input.iter().map(|&x| to_i16(x)));
            return;
        }
        self.buf.extend_from_slice(input);
        let end = self.base + self.buf.len() as u64;
        let half = self.half as i64;
        loop {
            let center = self.next.floor() as i64;
            // It needs input up to `half` samples past its center.
            if center + half >= end as i64 {
                break;
            }
            let frac = self.next - center as f64;
            let mut sum = 0.0f32;
            for j in (center - half + 1).max(self.base as i64)..=center + half {
                sum += self.buf[(j - self.base as i64) as usize] * self.kernel((j - center) as f64 - frac);
            }
            out.push(to_i16(sum));
            self.next += self.step;
        }
        // Keep what the next output's kernel reaches back to.
        let keep = (self.next.floor() as i64 - half + 1).max(self.base as i64) as u64;
        self.buf.drain(..(keep - self.base) as usize);
        self.base = keep;
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

    fn tone(rate: u32, hz: f64, secs: f64, amp: f32) -> Vec<f32> {
        let n = (f64::from(rate) * secs) as usize;
        (0..n).map(|i| amp * (2.0 * std::f64::consts::PI * hz * i as f64 / f64::from(rate)).sin() as f32).collect()
    }

    /// The level of `out`, past the filter's start-up, relative to full scale.
    fn rms(out: &[i16]) -> f64 {
        let s = &out[out.len() / 4..out.len() * 3 / 4];
        (s.iter().map(|&v| (f64::from(v) / 32767.0).powi(2)).sum::<f64>() / s.len() as f64).sqrt()
    }

    fn db(ratio: f64) -> f64 {
        20.0 * ratio.log10()
    }

    #[test]
    fn same_rate_passes_through() {
        let input: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
        let out = run(RATE, &input, 7);
        assert_eq!(out, input.iter().map(|&x| to_i16(x)).collect::<Vec<_>>());
    }

    #[test]
    fn keeps_duration_across_chunks() {
        for from in [48_000, 44_100, 96_000, 8_000] {
            for chunk in [441, 480, 512, 1000, 48_000] {
                let out = run(from, &vec![0.25f32; from as usize], chunk);
                // Less the kernel's reach ahead (`HALF_TAPS` output samples, twice that
                // upsampling), which comes out with the next chunk.
                let short = 16_000 - out.len() as i64;
                assert!((0..=50).contains(&short), "{from} chunk {chunk}: {}", out.len());
            }
        }
    }

    #[test]
    fn keeps_a_steady_level() {
        let out = run(48_000, &vec![0.5f32; 4800], 480);
        assert!(out[100..].iter().all(|&s| (s - to_i16(0.5)).abs() <= 40), "{:?}", &out[100..110]);
    }

    /// Speech up to 7 kHz (the consonants) comes through; what 16 kHz can't hold doesn't fold
    /// back into it.
    #[test]
    fn passes_speech_and_stops_aliases() {
        for from in [48_000, 44_100] {
            for hz in [300.0, 1000.0, 3000.0, 5000.0, 6500.0] {
                let level = db(rms(&run(from, &tone(from, hz, 0.5, 0.5), 512)) / (0.5 / 2f64.sqrt()));
                assert!(level.abs() < 0.5, "{from} Hz, {hz} Hz tone: {level:.2} dB");
            }
            for hz in [8800.0, 10_000.0, 13_000.0, 20_000.0] {
                let level = db(rms(&run(from, &tone(from, hz, 0.5, 0.5), 512)) / (0.5 / 2f64.sqrt()));
                assert!(level < -50.0, "{from} Hz, {hz} Hz tone: {level:.2} dB");
            }
        }
    }

    #[test]
    fn upsampling_keeps_a_tone() {
        let level = db(rms(&run(8_000, &tone(8_000, 1000.0, 0.5, 0.5), 160)) / (0.5 / 2f64.sqrt()));
        assert!(level.abs() < 0.5, "{level:.2} dB");
    }

    #[test]
    fn clamps() {
        assert_eq!(to_i16(2.0), i16::MAX);
        assert_eq!(to_i16(-2.0), -i16::MAX);
    }
}
