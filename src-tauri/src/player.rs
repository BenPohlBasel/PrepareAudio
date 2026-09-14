//! Vorhören für die Sync-Timeline.
//!
//! Ein eigener Thread besitzt den cpal-Ausgabestrom (CoreAudio) und füllt einen
//! kleinen Puffer mit gerenderten 100-ms-Blöcken. Gerendert wird genau die
//! Zuweisung der Clips: Stereo-Clips des ersten Senders links, des zweiten
//! rechts, Mono-Clips mittig; gelöschte und weggeschnittene Teile bleiben still.
//! Die Spuren werden über ihre Platzierung gelesen, also mit Versatz und Drift
//! sample-genau zueinander. Die Position geht mit etwa 30 Hz an die Oberfläche.

use crate::i18n::{t, tf, Msg};
use crate::sync::{self, Clip, ClipMode, Place, SyncPlan, Track};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;
use std::collections::{HashSet, VecDeque};
use std::f32::consts::FRAC_1_SQRT_2;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CHUNK_S: f64 = 0.1;
const BUFFER_S: f64 = 0.4;

/// Everything the renderer needs, as a snapshot of the current edit state.
#[derive(Clone)]
pub struct Mix {
    pub tracks: Vec<Track>,
    pub places: Vec<Place>,
    pub labels: Vec<String>,
    /// Linear preview gain per track.
    pub gains: Vec<f32>,
    pub clips: Vec<Clip>,
}

impl Mix {
    pub fn from_plan(plan: &SyncPlan) -> Self {
        Mix {
            tracks: plan.tracks.clone(),
            places: plan.places.clone(),
            labels: plan.labels.clone(),
            gains: plan.preview_gain_db.iter().map(|db| 10f32.powf(db / 20.0)).collect(),
            clips: plan.clips.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Position {
    /// Absolute timeline second.
    pub t: f64,
    pub playing: bool,
    pub error: Option<String>,
}

enum Cmd {
    SetMix(Arc<Mix>),
    Play(f64),
    Pause,
    Seek(f64),
    Solo { muted: HashSet<String>, solo: HashSet<String> },
}

pub struct Player {
    tx: Mutex<Sender<Cmd>>,
}

impl Player {
    pub fn start(on_position: impl Fn(Position) + Send + 'static) -> Player {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("preview".into())
            .spawn(move || run(rx, on_position))
            .expect("preview thread");
        Player { tx: Mutex::new(tx) }
    }

    fn send(&self, cmd: Cmd) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(cmd);
        }
    }

    pub fn set_mix(&self, mix: Mix) {
        self.send(Cmd::SetMix(Arc::new(mix)));
    }
    pub fn play(&self, t: f64) {
        self.send(Cmd::Play(t));
    }
    pub fn pause(&self) {
        self.send(Cmd::Pause);
    }
    pub fn seek(&self, t: f64) {
        self.send(Cmd::Seek(t));
    }
    pub fn set_solo(&self, muted: Vec<String>, solo: Vec<String>) {
        self.send(Cmd::Solo { muted: muted.into_iter().collect(), solo: solo.into_iter().collect() });
    }
}

struct Shared {
    buf: Mutex<VecDeque<f32>>,
    /// Frames the device actually played since the last reset.
    played: AtomicU64,
}

fn run(rx: Receiver<Cmd>, on_position: impl Fn(Position)) {
    let shared = Arc::new(Shared { buf: Mutex::new(VecDeque::new()), played: AtomicU64::new(0) });
    let mut mix: Option<Arc<Mix>> = None;
    let (mut muted, mut solo) = (HashSet::new(), HashSet::new());
    let mut stream: Option<cpal::Stream> = None;
    let mut rate = 48_000u32;
    let (mut t_start, mut t_render) = (0.0f64, 0.0f64);
    let mut playing = false;
    let mut last_emit = Instant::now();

    let reset = |t: f64, shared: &Shared, t_start: &mut f64, t_render: &mut f64| {
        if let Ok(mut b) = shared.buf.lock() {
            b.clear();
        }
        shared.played.store(0, Ordering::SeqCst);
        *t_start = t;
        *t_render = t;
    };
    let position = |shared: &Shared, t_start: f64, rate: u32| t_start + shared.played.load(Ordering::Relaxed) as f64 / rate as f64;

    loop {
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(Cmd::SetMix(m)) => {
                mix = Some(m);
                if playing {
                    let t = position(&shared, t_start, rate);
                    reset(t, &shared, &mut t_start, &mut t_render);
                }
            }
            Ok(Cmd::Play(t)) => {
                if stream.is_none() {
                    match open_stream(shared.clone()) {
                        Ok((s, r)) => {
                            stream = Some(s);
                            rate = r;
                        }
                        Err(e) => {
                            on_position(Position { t, playing: false, error: Some(e) });
                            continue;
                        }
                    }
                }
                reset(t, &shared, &mut t_start, &mut t_render);
                playing = true;
            }
            Ok(Cmd::Pause) => {
                if playing {
                    let t = position(&shared, t_start, rate);
                    stream = None;
                    playing = false;
                    reset(t, &shared, &mut t_start, &mut t_render);
                    on_position(Position { t, playing: false, error: None });
                }
            }
            Ok(Cmd::Seek(t)) => {
                reset(t, &shared, &mut t_start, &mut t_render);
                if !playing {
                    on_position(Position { t, playing: false, error: None });
                }
            }
            Ok(Cmd::Solo { muted: m, solo: s }) => {
                muted = m;
                solo = s;
                if playing {
                    let t = position(&shared, t_start, rate);
                    reset(t, &shared, &mut t_start, &mut t_render);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if !playing {
            continue;
        }
        let buffered = shared.buf.lock().map(|b| b.len() / 2).unwrap_or(0);
        if (buffered as f64) < BUFFER_S * rate as f64 {
            if let Some(m) = &mix {
                let frames = (CHUNK_S * rate as f64) as usize;
                let block = render(m, &muted, &solo, t_render, rate, frames);
                t_render += frames as f64 / rate as f64;
                if let Ok(mut b) = shared.buf.lock() {
                    b.extend(block);
                }
            }
        }
        if last_emit.elapsed() >= Duration::from_millis(33) {
            last_emit = Instant::now();
            on_position(Position { t: position(&shared, t_start, rate), playing: true, error: None });
        }
    }
}

fn open_stream(shared: Arc<Shared>) -> Result<(cpal::Stream, u32), String> {
    let device = cpal::default_host().default_output_device().ok_or(t(Msg::NoAudioOutput))?;
    let supported = device.default_output_config().map_err(|e| tf(Msg::AudioOutputError, &[("e", &e)]))?;
    if supported.sample_format() != cpal::SampleFormat::F32 {
        return Err(tf(Msg::AudioOutputFormat, &[("format", &format!("{:?}", supported.sample_format()))]));
    }
    let rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let config: cpal::StreamConfig = supported.into();
    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let frames = data.len() / channels.max(1);
                let mut filled = 0;
                if let Ok(mut buf) = shared.buf.try_lock() {
                    while filled < frames && buf.len() >= 2 {
                        let (l, r) = (buf.pop_front().unwrap_or(0.0), buf.pop_front().unwrap_or(0.0));
                        let frame = &mut data[filled * channels..(filled + 1) * channels];
                        if channels == 1 {
                            frame[0] = 0.5 * (l + r);
                        } else {
                            frame[0] = l;
                            frame[1] = r;
                            frame[2..].iter_mut().for_each(|x| *x = 0.0);
                        }
                        filled += 1;
                    }
                }
                data[filled * channels..].iter_mut().for_each(|x| *x = 0.0);
                shared.played.fetch_add(filled as u64, Ordering::Relaxed);
            },
            |err| eprintln!("preview stream: {err}"),
            None,
        )
        .map_err(|e| tf(Msg::AudioOutputError, &[("e", &e)]))?;
    stream.play().map_err(|e| tf(Msg::AudioOutputError, &[("e", &e)]))?;
    Ok((stream, rate))
}

/// Renders `frames` stereo frames of the edit state starting at timeline second `t0`.
pub fn render(mix: &Mix, muted: &HashSet<String>, solo: &HashSet<String>, t0: f64, rate: u32, frames: usize) -> Vec<f32> {
    let sr = rate as f64;
    let k0 = (t0 * sr).round() as i64;
    let k1 = k0 + frames as i64;
    let mut out = vec![0f32; frames * 2];
    let mut tmp: Vec<f32> = Vec::new();
    let stereo_possible = mix.labels.len() >= 2;
    for c in mix.clips.iter().filter(|c| !c.deleted) {
        let label = &mix.tracks[c.track].label;
        if muted.contains(label) || (!solo.is_empty() && !solo.contains(label)) {
            continue;
        }
        let pl = mix.places[c.track];
        let from = ((pl.to_timeline(c.t0) * sr).round() as i64).max(k0);
        let to = ((pl.to_timeline(c.t1) * sr).round() as i64).min(k1);
        if to <= from {
            continue;
        }
        tmp.clear();
        tmp.resize((to - from) as usize, 0.0);
        if sync::render_source(&mix.tracks, &mix.places, c.track, sr, from, to, &mut tmp).is_err() {
            continue;
        }
        let lane = mix.labels.iter().position(|l| l == label).unwrap_or(usize::MAX);
        let (gl, gr) = match (c.mode, lane) {
            (ClipMode::Stereo, 0) if stereo_possible => (1.0, 0.0),
            (ClipMode::Stereo, 1) => (0.0, 1.0),
            _ => (FRAC_1_SQRT_2, FRAC_1_SQRT_2),
        };
        let g = mix.gains.get(c.track).copied().unwrap_or(1.0);
        let offset = (from - k0) as usize;
        for (j, v) in tmp.iter().enumerate() {
            out[2 * (offset + j)] += v * g * gl;
            out[2 * (offset + j) + 1] += v * g * gr;
        }
    }
    for x in out.iter_mut() {
        let a = x.abs();
        if a > 0.8 {
            *x = x.signum() * (0.8 + 0.2 * ((a - 0.8) / 0.2).tanh());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn renders_the_assignment_of_each_clip() {
        let dir = tempdir("player");
        let sr = 8000u32;
        let four = sine(440.0, 0.5, sr, 0, 20 * sr as u64);
        write_float_wav(&dir.join("tracks/260101_S100000-E100020_D000020_4.wav"), sr, &four);
        write_float_wav(&dir.join("tracks/260101_S100005-E100025_D000020_5.wav"), sr, &sine(660.0, 0.5, sr, 0, 20 * sr as u64));
        let plan = sync::analyze(&[dir.join("tracks")], &AtomicBool::new(false), &mut |_| {}).unwrap();
        let mut mix = Mix::from_plan(&plan);
        mix.gains = vec![1.0; mix.tracks.len()];
        let p4 = mix.places[0].p;
        let none = HashSet::new();
        let at = |mix: &Mix, t: f64| render(mix, &none, &none, p4 + t, sr, 800);
        let expected = |k: usize| four[sr as usize + k];

        mix.clips = vec![Clip { track: 0, t0: 0.0, t1: 20.0, mode: ClipMode::Mono, deleted: false }];
        let mono = at(&mix, 1.0);
        for k in [0usize, 123, 799] {
            assert!((mono[2 * k] - expected(k) * FRAC_1_SQRT_2).abs() < 1e-5 && mono[2 * k] == mono[2 * k + 1]);
        }
        mix.clips[0].mode = ClipMode::Stereo;
        let st = at(&mix, 1.0);
        for k in [0usize, 123, 799] {
            let want = expected(k);
            let soft = if want.abs() > 0.8 { want.signum() * (0.8 + 0.2 * ((want.abs() - 0.8) / 0.2).tanh()) } else { want };
            assert!((st[2 * k] - soft).abs() < 1e-5 && st[2 * k + 1] == 0.0, "stereo clip of the first recorder plays left only");
        }
        mix.clips[0].deleted = true;
        assert!(at(&mix, 1.0).iter().all(|&v| v == 0.0), "deleted clips are silent");
        mix.clips[0].deleted = false;
        let muted: HashSet<String> = ["4".to_string()].into_iter().collect();
        assert!(render(&mix, &muted, &none, p4 + 1.0, sr, 800).iter().all(|&v| v == 0.0), "muted recorders are silent");
        mix.clips[0].t1 = 1.05;
        let trimmed = at(&mix, 1.0);
        assert!(trimmed[2 * 399] != 0.0 && trimmed[2 * 401..].iter().step_by(2).all(|&v| v == 0.0), "trimmed parts are silent");
    }

    /// Opens the default audio output and plays 0.2 s of silence (run by hand).
    #[test]
    #[ignore]
    fn opens_default_output() {
        let shared = Arc::new(Shared { buf: Mutex::new(VecDeque::from(vec![0f32; 2 * 9600])), played: AtomicU64::new(0) });
        let (stream, rate) = open_stream(shared.clone()).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        drop(stream);
        assert!(rate >= 8000 && shared.played.load(Ordering::Relaxed) > 0, "rate {rate}");
    }
}
