//! The **native viewscreen's saved presentation settings** (issue #1427) —
//! Bevy-free, and its location is injected.
//!
//! The browser viewscreen keeps its text size and contrast in its own
//! `localStorage`, which is what "on this endpoint" means for a page in a
//! browser. The native viewscreen cannot: its lobby document runs in an
//! Ultralight view whose storage session is **ephemeral and never written to
//! disk** (`panes::ultralight::UltralightHost::create`), so a `localStorage`
//! write there is forgotten the moment the process exits — which is precisely
//! what PRD #1418 story 12 says must not happen ("saved on that host machine
//! across sessions independently of the scenario").
//!
//! So on native the endpoint's store is a file this process owns:
//!
//! ```text
//! %APPDATA%\ProjectPhoenix\viewscreen-presentation.toml
//! ```
//!
//! and the page reaches it the only two ways a page can reach a host: the host
//! **seeds** the document with the saved record ([`presentation_script`],
//! injected exactly as [`super::panes::os_prefs::os_defaults_script`] is), and
//! the page **sends** a record back when the operator changes something
//! (`HostLobbyRecord::SetPresentation`, answered in
//! [`super::host_lobby::drain_surface_records`]).
//!
//! # What is stored, and what is not
//!
//! Two fields, each an `Option` whose `None` means *follow whatever this machine
//! says* — the same tri-state `gui/accessibility-profile.js` spells `default`.
//! The text size is WHOLE PERCENT rather than a float multiplier, because whole
//! percent is the resolution the slider actually offers (`TEXT_SCALE_STEP` is
//! `0.05`): an integer cannot be `NaN`, cannot drift through a TOML round trip,
//! and is what the operator reads on screen. The multiplier the CSS var wants is
//! derived once, where it is injected.
//!
//! That is the whole record. It carries no scenario, no save, no session, no
//! participant and no player's private profile, and this module has no path to
//! any of them: the file is one screen's reading comfort and nothing else, which
//! is what makes "a scenario load cannot disturb it, and it cannot enter a save"
//! true by construction rather than by care.
//!
//! # Three properties, each a rule rather than an implementation
//!
//! 1. **The location is injectable.** A [`ViewscreenPresentationStore`] is a
//!    path and nothing else. [`ViewscreenPresentationStore::user`] resolves the
//!    operator's real settings directory; every other constructor takes one, so
//!    the tests below run against a scratch directory and never touch a
//!    developer's own display settings. Exactly [`super::layout_store`]'s shape,
//!    for exactly its reason.
//! 2. **A write is atomic**, through the same
//!    [`crate::native_file::write_preferences`] the saved bridge layouts use: a
//!    host killed mid-write leaves the old settings or the new ones, never half
//!    a TOML file that the next boot reports as corrupt.
//! 3. **A saved file is never trusted.** [`ViewscreenPresentationStore::load`]
//!    parses *and* re-clamps. A hand-edited `text_scale_percent = 4000` is a
//!    request this build has verified nothing at, so it is clamped into range
//!    rather than honoured into a lobby nobody can read — and a file that is not
//!    valid TOML at all reads as "nothing saved", because a display that comes
//!    up at its default is a recoverable Tuesday and a host that refuses to boot
//!    is not.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::setup_accessibility::{SUPPORTED_TEXT_SCALE_MAX, SUPPORTED_TEXT_SCALE_MIN};

/// The file the settings live in, inside the store's directory.
const FILE_NAME: &str = "viewscreen-presentation.toml";

/// The endpoint's saved presentation settings.
///
/// `None` is *follow this machine* in both fields — not "off". The distinction
/// is the whole point of the tri-state the page offers: an operator must be able
/// to force standard contrast on a display whose OS asked for more, which is a
/// different record from never having chosen.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ViewscreenPresentation {
    /// The chosen text size as WHOLE PERCENT (`150` is 1.5x), already clamped to
    /// the supported range.
    pub text_scale_percent: Option<u32>,
    /// `Some(true)` forces higher contrast, `Some(false)` forces standard.
    pub contrast: Option<bool>,
    /// Camera/page shake intensity as WHOLE PERCENT (issue #1428): `0` is off,
    /// `100` is the shipped magnitude, `None` follows this machine's motion
    /// preference. Whole percent for the reason the text size is: it is the
    /// number the operator reads on screen, and an integer cannot be `NaN`.
    pub shake_percent: Option<u32>,
    /// Shield-flash intensity as whole percent (issue #1428).
    pub flash_percent: Option<u32>,
    /// Decorative-motion intensity as whole percent (issue #1428).
    pub decorative_motion_percent: Option<u32>,
}

/// The supported text-size range in whole percent, derived from the one
/// multiplier contract (`gui/accessibility-profile.js` and its Rust mirror
/// `setup_accessibility::SUPPORTED_TEXT_SCALE_MIN`/`MAX`) rather than restated —
/// so raising the ceiling stays the single edit issue #1422 made it.
pub const MIN_TEXT_SCALE_PERCENT: u32 = (SUPPORTED_TEXT_SCALE_MIN * 100.0) as u32;
/// The largest text size this build claims the viewscreen chrome reflows at.
pub const MAX_TEXT_SCALE_PERCENT: u32 = (SUPPORTED_TEXT_SCALE_MAX * 100.0) as u32;

/// The largest visual-effect intensity: an effect at 100% is the shipped one,
/// and there is no "louder than shipped" (issue #1428). Mirrors
/// `EFFECT_FULL` in `gui/visual-effects.js`.
pub const MAX_EFFECT_PERCENT: u32 = 100;

/// The TOML shape on disk. Separate from [`ViewscreenPresentation`] so the file
/// format is a decision this module can change without every caller learning
/// about it, and so a missing field is a `None` rather than a parse failure.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedPresentation {
    #[serde(default)]
    audio: Option<toml::Value>,
    #[serde(default)]
    text_scale_percent: Option<u32>,
    #[serde(default)]
    contrast: Option<bool>,
    #[serde(default)]
    shake_percent: Option<u32>,
    #[serde(default)]
    flash_percent: Option<u32>,
    #[serde(default)]
    decorative_motion_percent: Option<u32>,
}

impl ViewscreenPresentation {
    /// The record with every field at its documented default — the whole of
    /// "Reset all" on this side, and its scope is structural: there is nothing
    /// else in the record to reset.
    pub fn following_system() -> Self {
        Self::default()
    }

    /// True when nothing has been chosen explicitly on this display.
    pub fn is_default(&self) -> bool {
        self.text_scale_percent.is_none()
            && self.contrast.is_none()
            && self.effect_percents().iter().all(Option::is_none)
    }

    /// The three effect intensities in display order (issue #1428), so a caller
    /// iterating them cannot miss one the way a hand-written list can.
    pub fn effect_percents(&self) -> [Option<u32>; 3] {
        [
            self.shake_percent,
            self.flash_percent,
            self.decorative_motion_percent,
        ]
    }

    /// The record with its text size clamped into the range this build supports.
    ///
    /// An out-of-range value is CLAMPED rather than dropped, mirroring
    /// `clampTextScale` in `gui/accessibility-profile.js`: a record asking for
    /// 300% wants the largest size this fleet claims to reflow at, and silently
    /// returning it to 100% would read as the setting not working at all.
    pub fn sanitised(self) -> Self {
        // An effect intensity is a percentage of an effect, so its ceiling is
        // 100 — a hand-edited `400` is a request for the largest this build
        // renders, exactly as an out-of-range text size is (issue #1428).
        let effect = |percent: Option<u32>| percent.map(|value| value.min(MAX_EFFECT_PERCENT));
        Self {
            text_scale_percent: self
                .text_scale_percent
                .map(|percent| percent.clamp(MIN_TEXT_SCALE_PERCENT, MAX_TEXT_SCALE_PERCENT)),
            contrast: self.contrast,
            shake_percent: effect(self.shake_percent),
            flash_percent: effect(self.flash_percent),
            decorative_motion_percent: effect(self.decorative_motion_percent),
        }
    }

    /// The multiplier the CSS var wants, derived from the stored percent.
    pub fn text_scale(&self) -> Option<f64> {
        self.sanitised()
            .text_scale_percent
            .map(|percent| f64::from(percent) / 100.0)
    }

    /// The file body this record is saved as.
    fn to_toml(self) -> String {
        let mut out = String::from(
            "# Project Phoenix — this display's own text size and contrast.\n\
             # Written by the Viewscreen settings menu (Display tab). A field that is\n\
             # absent follows this machine's own system preference.\n",
        );
        if let Some(percent) = self.text_scale_percent {
            out.push_str(&format!("text_scale_percent = {percent}\n"));
        }
        if let Some(contrast) = self.contrast {
            out.push_str(&format!("contrast = {contrast}\n"));
        }
        for (key, percent) in [
            ("shake_percent", self.shake_percent),
            ("flash_percent", self.flash_percent),
            ("decorative_motion_percent", self.decorative_motion_percent),
        ] {
            if let Some(percent) = percent {
                out.push_str(&format!("{key} = {percent}\n"));
            }
        }
        out
    }
}

/// The JavaScript that seeds the native lobby page with the saved record.
///
/// One assignment to `window.PhoenixViewscreenPresentation`, whose keys are
/// exactly the fields `gui/viewscreen-presentation.js`'s
/// `readInjectedViewscreenPresentation` normalises — with `null` for
/// *follow this machine*, which its `normalizeViewscreenPresentation` reads as
/// the follow-the-system default. Injected into the lobby document's `<head>`
/// before any `gui/` module evaluates, so the menus and the lobby chrome come up
/// at the saved size rather than flashing through the default first.
///
/// Deliberately hand-formatted rather than routed through `serde_json`, which
/// AGENTS.md rule 1 confines to `core::codec`.
///
/// # Injection-safety invariant
///
/// This string is interpolated verbatim into a `<script>` inside the lobby's
/// HTML, so it is injection-safe **only because every field is a `bool`, a
/// finite clamped number, or `null`** — none can contain a quote, a backslash, a
/// newline, `</script>` or `${`. If this record ever grows a **string** field, it
/// MUST be routed through a JSON string escaper before it reaches this `format!`;
/// an unescaped string here is an HTML/JS injection sink. The same warning sits
/// over [`super::panes::os_prefs::os_defaults_script`], and for the same reason.
pub fn presentation_script(record: &ViewscreenPresentation) -> String {
    let sane = record.sanitised();
    let scale = match sane.text_scale() {
        Some(value) => format_scale(value),
        None => "null".to_string(),
    };
    let contrast = match sane.contrast {
        Some(true) => "true",
        Some(false) => "false",
        None => "null",
    };
    // The three effects cross as `0..=1` FRACTIONS, the shape
    // `readInjectedViewscreenPresentation` normalises and the same shape the
    // text scale crosses in — a multiplier, not the stored percent.
    let fraction = |percent: Option<u32>| match percent {
        Some(value) => format_scale(f64::from(value.min(MAX_EFFECT_PERCENT)) / 100.0),
        None => "null".to_string(),
    };
    let shake = fraction(sane.shake_percent);
    let flash = fraction(sane.flash_percent);
    let decorative = fraction(sane.decorative_motion_percent);
    format!(
        "window.PhoenixViewscreenPresentation = \
         {{\"textScale\":{scale},\"contrast\":{contrast},\"shake\":{shake},\
         \"flash\":{flash},\"decorativeMotion\":{decorative}}};"
    )
}

/// Current reading preferences for the separate native HUD document. The same
/// endpoint override and OS default layer as the lobby; no second saved record.
pub fn hud_reading_script(
    record: &ViewscreenPresentation,
    os: &super::panes::os_prefs::OsAccessibilityPrefs,
) -> String {
    let sane = record.sanitised();
    let scale = sane.text_scale().unwrap_or(f64::from(os.text_scale));
    let scale = if scale.is_finite() {
        scale.clamp(SUPPORTED_TEXT_SCALE_MIN, SUPPORTED_TEXT_SCALE_MAX)
    } else {
        1.0
    };
    format!(
        "window.__phoenixSetHudReading({{textScale:{},contrast:{}}})",
        format_scale(scale),
        sane.contrast.unwrap_or(os.high_contrast)
    )
}

/// Format a scale as a JS number literal with no trailing-dot noise (`1` and
/// `1.25`, never `1.` or an empty string). The twin of `os_prefs::format_scale`,
/// which is private to that module.
fn format_scale(scale: f64) -> String {
    let mut s = format!("{scale}");
    if s.ends_with('.') {
        s.push('0');
    }
    s
}

/// A directory holding this machine's saved viewscreen settings.
#[derive(Debug, Clone)]
pub struct ViewscreenPresentationStore {
    dir: PathBuf,
}

impl ViewscreenPresentationStore {
    /// The store in the operator's own settings directory, or `None` where this
    /// platform will not name one.
    ///
    /// `BaseDirs::data_dir()` and the same `ProjectPhoenix` folder
    /// [`super::layout_store::LayoutStore::user`] uses, so an operator browsing
    /// their settings finds one folder rather than two.
    pub fn user() -> Option<Self> {
        let base = directories::BaseDirs::new()?;
        Some(Self {
            dir: base.data_dir().join("ProjectPhoenix"),
        })
    }

    /// A store rooted at `dir` — the constructor tests use.
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The file this store reads and writes.
    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    /// The saved record, or the follow-the-system default.
    ///
    /// Every failure — no file, unreadable file, invalid TOML, an unknown key
    /// from a newer build — reads as "nothing saved". A display that comes up at
    /// its default is recoverable in one press; a host that refuses to start
    /// because of a comfort setting is not.
    pub fn load(&self) -> ViewscreenPresentation {
        let Ok(text) = std::fs::read_to_string(self.path()) else {
            return ViewscreenPresentation::following_system();
        };
        let Ok(saved) = toml::from_str::<SavedPresentation>(&text) else {
            return ViewscreenPresentation::following_system();
        };
        ViewscreenPresentation {
            text_scale_percent: saved.text_scale_percent,
            contrast: saved.contrast,
            shake_percent: saved.shake_percent,
            flash_percent: saved.flash_percent,
            decorative_motion_percent: saved.decorative_motion_percent,
        }
        .sanitised()
    }

    /// Write the record, atomically, creating the directory if it is not there.
    ///
    /// A record that is entirely at its defaults REMOVES the file rather than
    /// writing an empty one: "the operator reset this display" and "this machine
    /// has never had a display setting" are the same state, and leaving a stub
    /// behind makes the next reader wonder which.
    pub fn save(&self, record: &ViewscreenPresentation) -> std::io::Result<()> {
        let audio = std::fs::read_to_string(self.path())
            .ok()
            .and_then(|text| toml::from_str::<SavedPresentation>(&text).ok())
            .and_then(|record| record.audio);
        self.save_sections(record, audio)
    }

    /// Merge audio with the existing endpoint record. Hardware output identities
    /// are deliberately stored by bridge-media instead, and no cues are stored.
    pub fn save_audio(&self, mix: super::audio::mix::AudioMix) -> std::io::Result<()> {
        self.save_audio_comfort(
            mix,
            self.load_audio_mono(),
            self.load_audio_ducking(),
            self.load_audio_reduced_range(),
        )
    }

    pub fn save_audio_mono(
        &self,
        mix: super::audio::mix::AudioMix,
        mono: bool,
    ) -> std::io::Result<()> {
        self.save_audio_comfort(
            mix,
            mono,
            self.load_audio_ducking(),
            self.load_audio_reduced_range(),
        )
    }

    pub fn save_audio_preferences(
        &self,
        mix: super::audio::mix::AudioMix,
        ducking: bool,
    ) -> std::io::Result<()> {
        self.save_audio_comfort(
            mix,
            self.load_audio_mono(),
            ducking,
            self.load_audio_reduced_range(),
        )
    }

    pub fn save_audio_comfort(
        &self,
        mix: super::audio::mix::AudioMix,
        mono: bool,
        ducking: bool,
        reduced_range: bool,
    ) -> std::io::Result<()> {
        // Preserve independently owned audio options as well as display fields.
        let mut audio = std::fs::read_to_string(self.path())
            .ok()
            .and_then(|text| toml::from_str::<SavedPresentation>(&text).ok())
            .and_then(|record| record.audio)
            .filter(toml::Value::is_table)
            .unwrap_or_else(|| toml::Value::Table(toml::Table::new()));
        let table = audio.as_table_mut().expect("table checked above");
        table.insert("version".into(), toml::Value::Integer(1));
        table.insert(
            "mix".into(),
            toml::Value::try_from(mix.sanitised()).map_err(std::io::Error::other)?,
        );
        table.insert("mono".into(), toml::Value::Boolean(mono));
        table.insert("ducking".into(), toml::Value::Boolean(ducking));
        table.insert("reducedRange".into(), toml::Value::Boolean(reduced_range));
        self.save_sections(&self.load(), Some(audio))
    }

    pub fn load_audio_mono(&self) -> bool {
        std::fs::read_to_string(self.path())
            .ok()
            .and_then(|text| toml::from_str::<SavedPresentation>(&text).ok())
            .and_then(|record| record.audio)
            .and_then(|audio| audio.try_into::<SavedAudio>().ok())
            .filter(|audio| audio.version == 1)
            .map(|audio| audio.mono)
            .unwrap_or(false)
    }

    pub fn load_audio_ducking(&self) -> bool {
        std::fs::read_to_string(self.path())
            .ok()
            .and_then(|text| toml::from_str::<SavedPresentation>(&text).ok())
            .and_then(|saved| saved.audio)
            .and_then(|audio| audio.try_into::<SavedAudio>().ok())
            .is_some_and(|audio| audio.version == 1 && audio.ducking)
    }

    pub fn load_audio_reduced_range(&self) -> bool {
        std::fs::read_to_string(self.path())
            .ok()
            .and_then(|text| toml::from_str::<SavedPresentation>(&text).ok())
            .and_then(|saved| saved.audio)
            .and_then(|audio| audio.try_into::<SavedAudio>().ok())
            .is_some_and(|audio| audio.version == 1 && audio.reduced_range)
    }

    pub fn load_audio(&self) -> (super::audio::mix::AudioMix, &'static str) {
        let text = match std::fs::read_to_string(self.path()) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return (Default::default(), "saved");
            }
            Err(_) => return (Default::default(), "unavailable"),
        };
        let saved = match toml::from_str::<SavedPresentation>(&text) {
            Ok(saved) => saved,
            Err(_) => return (Default::default(), "corrupt"),
        };
        let Some(audio) = saved.audio else {
            return (Default::default(), "saved");
        };
        match audio.try_into::<SavedAudio>() {
            Ok(record) if record.version == 1 => (record.mix.sanitised(), "saved"),
            _ => (Default::default(), "corrupt"),
        }
    }

    fn save_sections(
        &self,
        record: &ViewscreenPresentation,
        audio: Option<toml::Value>,
    ) -> std::io::Result<()> {
        let sane = record.sanitised();
        let path = self.path();
        if sane.is_default() && audio.is_none() {
            return match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            };
        }
        create_dir(&self.dir)?;
        let mut table: toml::Table =
            toml::from_str(&sane.to_toml()).map_err(std::io::Error::other)?;
        if let Some(audio) = audio {
            table.insert("audio".into(), audio);
        }
        let text = toml::to_string(&table).map_err(std::io::Error::other)?;
        crate::native_file::write_preferences(&path, &text)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedAudio {
    version: u32,
    mix: super::audio::mix::AudioMix,
    #[serde(default)]
    mono: bool,
    #[serde(default)]
    ducking: bool,
    #[serde(default, rename = "reducedRange")]
    reduced_range: bool,
}

fn create_dir(dir: &Path) -> std::io::Result<()> {
    match std::fs::create_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
#[path = "viewscreen_presentation_tests.rs"]
mod tests;
