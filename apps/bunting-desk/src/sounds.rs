//! Alert sounds. RIT 1.8.466 sounds the system "Exclamation" when news
//! arrives and for some toasts, and has a speaker toggle in its status
//! bar (`docs/research/rit-binary-audit/desk-client-inventory-1.8.466.md`,
//! "Alerts and sounds"). The owner asked for "sounds for order filled,
//! etc" as well, so the desk also sounds fills, rejects, tender offers
//! and a lost connection (Bunting-added).
//!
//! The tones are synthesized here, so no RIT or operating-system asset
//! is copied. Each is written once as a WAV file and played by the
//! platform's own player, so the app links no audio library:
//! - `afplay` on macOS;
//! - `PipeWire`, `PulseAudio` or ALSA (`pw-play`, `paplay`, `aplay`) on
//!   Linux;
//! - the .NET `SoundPlayer` on Windows.
//!
//! The player runs off the UI thread, and a missing player stays silent.
//! Which events sound is decided by [`Watch`], a pure diff of the model,
//! so the rules are unit-tested.
#![allow(
    clippy::float_arithmetic,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "tone synthesis is sample arithmetic in f32, clamped to i16"
)]

use bunting_trading_state::{Connection, Model, OrderStatus};
use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Something the trader should hear.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Cue {
    /// One of your orders traded.
    Fill,
    /// The venue refused one of your orders.
    Reject,
    /// A news item arrived (RIT: Exclamation).
    News,
    /// A tender offer arrived.
    Tender,
    /// The session to the venue dropped.
    Disconnected,
}

impl Cue {
    const ALL: [Self; 5] = [
        Self::Fill,
        Self::Reject,
        Self::News,
        Self::Tender,
        Self::Disconnected,
    ];

    const fn file_name(self) -> &'static str {
        match self {
            Self::Fill => "fill.wav",
            Self::Reject => "reject.wav",
            Self::News => "news.wav",
            Self::Tender => "tender.wav",
            Self::Disconnected => "disconnected.wav",
        }
    }

    /// Notes as `(frequency Hz, milliseconds)`; a zero frequency rests.
    const fn notes(self) -> &'static [(f32, u32)] {
        match self {
            // A quick rising pair: done, in your favour or not.
            Self::Fill => &[(1046.5, 55), (1568.0, 90)],
            // Two low, flat beats.
            Self::Reject => &[(233.1, 110), (0., 50), (233.1, 140)],
            // A soft single bell.
            Self::News => &[(880.0, 260)],
            // A rising three-note call: something waits for an answer.
            Self::Tender => &[(659.3, 90), (784.0, 90), (1046.5, 180)],
            // Falling: the line went quiet.
            Self::Disconnected => &[(784.0, 120), (587.3, 120), (392.0, 240)],
        }
    }
}

/// What the model had already shown when the last cues were taken, so
/// only new events sound and joining a venue never replays history.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Watch {
    fill_sequence: u64,
    news_id: Option<u128>,
    tender_sequence: u64,
    rejected_order: u64,
    connected: bool,
}

impl Watch {
    /// The marks of everything `model` holds now.
    #[must_use]
    pub fn of(model: &Model) -> Self {
        Self {
            fill_sequence: model.fills().back().map_or(0, |fill| fill.sequence),
            news_id: model.news().back().map(|item| item.news_id),
            tender_sequence: model
                .tenders()
                .iter()
                .map(|tender| tender.sequence)
                .max()
                .unwrap_or(0),
            rejected_order: model
                .orders()
                .iter()
                .filter(|order| matches!(order.status, OrderStatus::Rejected(_)))
                .map(|order| order.client_order_id)
                .max()
                .unwrap_or(0),
            connected: matches!(model.connection(), Connection::Connected { .. }),
        }
    }

    /// The cues for what `model` shows beyond these marks, each kind at
    /// most once, and the marks moved up to it.
    pub fn cues(&mut self, model: &Model) -> Vec<Cue> {
        let now = Self::of(model);
        let mut cues = Vec::new();
        if now.fill_sequence > self.fill_sequence {
            cues.push(Cue::Fill);
        }
        if now.rejected_order > self.rejected_order {
            cues.push(Cue::Reject);
        }
        if now.news_id.is_some() && now.news_id != self.news_id {
            cues.push(Cue::News);
        }
        if now.tender_sequence > self.tender_sequence {
            cues.push(Cue::Tender);
        }
        if self.connected && !now.connected {
            cues.push(Cue::Disconnected);
        }
        *self = Self {
            fill_sequence: now.fill_sequence.max(self.fill_sequence),
            tender_sequence: now.tender_sequence.max(self.tender_sequence),
            rejected_order: now.rejected_order.max(self.rejected_order),
            ..now
        };
        cues
    }
}

const SAMPLE_RATE: u32 = 44_100;

/// `cue` as a mono 16-bit PCM WAV file: each note a sine with a
/// partial at twice its pitch, a 5 ms attack and an exponential decay,
/// at a moderate level.
#[must_use]
pub fn wav(cue: Cue) -> Vec<u8> {
    let mut samples: Vec<i16> = Vec::new();
    for &(frequency, millis) in cue.notes() {
        let count = (SAMPLE_RATE * millis / 1000) as usize;
        let attack = (SAMPLE_RATE / 200) as f32;
        for index in 0..count {
            let t = index as f32 / SAMPLE_RATE as f32;
            if frequency <= 0. {
                samples.push(0);
                continue;
            }
            let envelope = (index as f32 / attack).min(1.) * (-t * 9.).exp();
            let phase = std::f32::consts::TAU * frequency * t;
            let value = (phase.sin() + 0.25 * (2. * phase).sin()) * envelope * 0.32;
            samples.push((value.clamp(-1., 1.) * f32::from(i16::MAX)) as i16);
        }
    }
    let data_len = u32::try_from(samples.len() * 2).unwrap_or(u32::MAX);
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1_u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

/// The saved switch, beside the other desk settings.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct File {
    enabled: bool,
}

/// Sounds on or off, with the place they are saved and the WAVs
/// written so far.
pub struct Sounds {
    enabled: bool,
    settings: PathBuf,
    cache: PathBuf,
    last: [Option<Instant>; Cue::ALL.len()],
}

impl Global for Sounds {}

/// The same cue never sounds twice within this, so a burst of fills is
/// one sound.
const REPEAT: Duration = Duration::from_millis(250);

impl Sounds {
    /// Sounds saved in `settings` (on when there is no file), with tones
    /// written under `cache`.
    #[must_use]
    pub fn at(settings: PathBuf, cache: PathBuf) -> Self {
        let enabled = std::fs::read_to_string(&settings)
            .ok()
            .and_then(|text| serde_json::from_str::<File>(&text).ok())
            .is_none_or(|file| file.enabled);
        Self {
            enabled,
            settings,
            cache,
            last: [None; Cue::ALL.len()],
        }
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Turns sounds on or off and saves the choice.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        let text = serde_json::to_string(&File { enabled }).unwrap_or_default();
        if let Some(dir) = self.settings.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(error) = std::fs::write(&self.settings, text) {
            eprintln!(
                "bunting-desk: cannot save {}: {error}",
                self.settings.display()
            );
        }
    }

    /// Plays each of `cues` unless sounds are off or it just played.
    pub fn play(&mut self, cues: &[Cue]) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        for cue in cues {
            let slot = &mut self.last[*cue as usize];
            if slot.is_some_and(|at| now.duration_since(at) < REPEAT) {
                continue;
            }
            *slot = Some(now);
            if let Some(path) = self.tone(*cue) {
                spawn_player(path);
            }
        }
    }

    /// The WAV for `cue`, written on first use.
    fn tone(&self, cue: Cue) -> Option<PathBuf> {
        let path = self.cache.join(cue.file_name());
        if !path.exists() {
            std::fs::create_dir_all(&self.cache).ok()?;
            std::fs::write(&path, wav(cue)).ok()?;
        }
        Some(path)
    }
}

/// Installs the sound settings from the desk's config directory.
pub fn install(cx: &mut App) {
    let dir = crate::appearance::desk_config_dir();
    cx.set_global(Sounds::at(dir.join("sounds.json"), dir.join("sounds")));
}

/// Plays `cues` through the installed settings, if any.
pub fn play(cues: &[Cue], cx: &mut App) {
    if !cues.is_empty() && cx.has_global::<Sounds>() {
        cx.global_mut::<Sounds>().play(cues);
    }
}

/// Whether sounds are on (true before [`install`] runs).
#[must_use]
pub fn enabled(cx: &App) -> bool {
    !cx.has_global::<Sounds>() || cx.global::<Sounds>().enabled()
}

/// Flips sounds on or off.
pub fn toggle(cx: &mut App) {
    if cx.has_global::<Sounds>() {
        let sounds = cx.global_mut::<Sounds>();
        let enabled = !sounds.enabled();
        sounds.set_enabled(enabled);
    }
}

/// Starts the platform player on `path` and reaps it on its own thread.
fn spawn_player(path: PathBuf) {
    std::thread::spawn(move || {
        let Some(mut command) = player_command(&path) else {
            return;
        };
        if let Ok(mut child) = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            let _ = child.wait();
        }
    });
}

fn player_command(path: &Path) -> Option<Command> {
    if cfg!(target_os = "macos") {
        let mut command = Command::new("afplay");
        command.arg(path);
        return Some(command);
    }
    if cfg!(target_os = "windows") {
        let script = format!(
            "(New-Object Media.SoundPlayer '{}').PlaySync()",
            path.display().to_string().replace('\'', "''")
        );
        let mut command = Command::new("powershell");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        return Some(command);
    }
    ["pw-play", "paplay", "aplay"]
        .into_iter()
        .find(|program| on_path(program))
        .map(|program| {
            let mut command = Command::new(program);
            if program == "aplay" {
                command.arg("-q");
            }
            command.arg(path);
            command
        })
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cue_is_a_valid_short_pcm_wav() {
        for cue in Cue::ALL {
            let bytes = wav(cue);
            assert_eq!(&bytes[..4], b"RIFF");
            assert_eq!(&bytes[8..16], b"WAVEfmt ");
            let data = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as usize;
            assert_eq!(bytes.len(), 44 + data, "{cue:?} header length");
            let seconds = data as f32 / (SAMPLE_RATE as f32 * 2.);
            assert!((0.05..0.6).contains(&seconds), "{cue:?} lasts {seconds} s");
        }
    }

    #[test]
    fn a_fresh_model_has_nothing_to_sound() {
        let model = Model::new(1);
        let mut watch = Watch::of(&model);
        assert!(watch.cues(&model).is_empty());
    }

    #[test]
    fn muted_sounds_play_nothing_and_the_switch_is_saved() {
        let dir = std::env::temp_dir().join(format!("desk-sounds-{}", std::process::id()));
        let settings = dir.join("sounds.json");
        let mut sounds = Sounds::at(settings.clone(), dir.join("tones"));
        assert!(sounds.enabled(), "on by default, as RIT ships");
        sounds.set_enabled(false);
        sounds.play(&[Cue::Fill]);
        assert!(!dir.join("tones").exists(), "nothing written while muted");
        assert!(!Sounds::at(settings, dir.join("tones")).enabled());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
