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
//!    [`super::layout_store::write_atomically`] the saved bridge layouts use: a
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
}

/// The supported text-size range in whole percent, derived from the one
/// multiplier contract (`gui/accessibility-profile.js` and its Rust mirror
/// `setup_accessibility::SUPPORTED_TEXT_SCALE_MIN`/`MAX`) rather than restated —
/// so raising the ceiling stays the single edit issue #1422 made it.
pub const MIN_TEXT_SCALE_PERCENT: u32 = (SUPPORTED_TEXT_SCALE_MIN * 100.0) as u32;
/// The largest text size this build claims the viewscreen chrome reflows at.
pub const MAX_TEXT_SCALE_PERCENT: u32 = (SUPPORTED_TEXT_SCALE_MAX * 100.0) as u32;

/// The TOML shape on disk. Separate from [`ViewscreenPresentation`] so the file
/// format is a decision this module can change without every caller learning
/// about it, and so a missing field is a `None` rather than a parse failure.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedPresentation {
    #[serde(default)]
    text_scale_percent: Option<u32>,
    #[serde(default)]
    contrast: Option<bool>,
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
        self.text_scale_percent.is_none() && self.contrast.is_none()
    }

    /// The record with its text size clamped into the range this build supports.
    ///
    /// An out-of-range value is CLAMPED rather than dropped, mirroring
    /// `clampTextScale` in `gui/accessibility-profile.js`: a record asking for
    /// 300% wants the largest size this fleet claims to reflow at, and silently
    /// returning it to 100% would read as the setting not working at all.
    pub fn sanitised(self) -> Self {
        Self {
            text_scale_percent: self
                .text_scale_percent
                .map(|percent| percent.clamp(MIN_TEXT_SCALE_PERCENT, MAX_TEXT_SCALE_PERCENT)),
            contrast: self.contrast,
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
    format!(
        "window.PhoenixViewscreenPresentation = \
         {{\"textScale\":{scale},\"contrast\":{contrast}}};"
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
        let sane = record.sanitised();
        let path = self.path();
        if sane.is_default() {
            return match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            };
        }
        create_dir(&self.dir)?;
        super::layout_store::write_atomically(&path, &sane.to_toml())
    }
}

fn create_dir(dir: &Path) -> std::io::Result<()> {
    match std::fs::create_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("phoenix-viewscreen-presentation-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn an_untouched_machine_follows_its_own_system_preferences() {
        let store = ViewscreenPresentationStore::at(scratch("absent"));
        assert_eq!(store.load(), ViewscreenPresentation::following_system());
        assert!(store.load().is_default());
    }

    #[test]
    fn a_saved_record_survives_the_process_that_wrote_it() {
        // The whole claim of the slice on this side: the operator turns the
        // shared screen up, the host restarts, and the room is still readable.
        let dir = scratch("round-trip");
        let store = ViewscreenPresentationStore::at(&dir);
        let chosen = ViewscreenPresentation {
            text_scale_percent: Some(150),
            contrast: Some(true),
        };
        store.save(&chosen).expect("save");

        // A second store over the same directory is the next launch: nothing of
        // the first one's memory is available to it except the file.
        let next_launch = ViewscreenPresentationStore::at(&dir);
        assert_eq!(next_launch.load(), chosen);
    }

    #[test]
    fn each_field_is_saved_and_reset_on_its_own() {
        // Per-setting reset (PRD #1418 story 17): returning contrast to the
        // system must leave the text size exactly where the operator put it.
        let dir = scratch("per-setting");
        let store = ViewscreenPresentationStore::at(&dir);
        store
            .save(&ViewscreenPresentation {
                text_scale_percent: Some(175),
                contrast: Some(false),
            })
            .expect("save");
        let mut record = store.load();
        record.contrast = None;
        store.save(&record).expect("save");

        let reloaded = store.load();
        assert_eq!(reloaded.text_scale_percent, Some(175));
        assert_eq!(reloaded.contrast, None);
    }

    #[test]
    fn resetting_everything_leaves_no_file_behind() {
        let dir = scratch("reset-all");
        let store = ViewscreenPresentationStore::at(&dir);
        store
            .save(&ViewscreenPresentation {
                text_scale_percent: Some(200),
                contrast: Some(true),
            })
            .expect("save");
        assert!(store.path().exists());

        store
            .save(&ViewscreenPresentation::following_system())
            .expect("reset");
        assert!(!store.path().exists());
        assert_eq!(store.load(), ViewscreenPresentation::following_system());
        // …and resetting twice is not an error, because an operator pressing it
        // again is not doing anything wrong.
        store
            .save(&ViewscreenPresentation::following_system())
            .expect("reset again");
    }

    #[test]
    fn a_hand_edited_file_is_clamped_rather_than_honoured() {
        let dir = scratch("hand-edited");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(
            dir.join(FILE_NAME),
            "text_scale_percent = 4000\ncontrast = true\n",
        )
        .expect("write");
        let store = ViewscreenPresentationStore::at(&dir);
        // 40x is a size nothing in this fleet has verified a layout at; the
        // largest supported one is what the operator gets.
        assert_eq!(
            store.load().text_scale_percent,
            Some(MAX_TEXT_SCALE_PERCENT)
        );
        assert_eq!(store.load().contrast, Some(true));
    }

    #[test]
    fn a_corrupt_or_foreign_file_reads_as_nothing_saved() {
        let dir = scratch("corrupt");
        std::fs::create_dir_all(&dir).expect("dir");
        for body in ["this is not toml at all {{{", "unexpected_key = 3\n"] {
            std::fs::write(dir.join(FILE_NAME), body).expect("write");
            let store = ViewscreenPresentationStore::at(&dir);
            assert_eq!(store.load(), ViewscreenPresentation::following_system());
        }
    }

    #[test]
    fn the_seed_script_is_one_well_formed_assignment_with_no_injection_sink() {
        let script = presentation_script(&ViewscreenPresentation {
            text_scale_percent: Some(125),
            contrast: Some(false),
        });
        assert_eq!(
            script,
            "window.PhoenixViewscreenPresentation = {\"textScale\":1.25,\"contrast\":false};"
        );
        // The invariant the module note states, asserted rather than trusted:
        // nothing that could close the script element or open a template can
        // reach the injected string, because every field is a number or a bool.
        assert!(!script.contains("</script"));
        assert!(!script.contains("${"));
        assert!(!script.contains('\n'));
    }

    #[test]
    fn a_machine_with_nothing_chosen_seeds_nulls_rather_than_values() {
        // `null`, not `1` and `false`: the page must be able to tell "follow this
        // machine" from "somebody chose 100% and standard contrast", because the
        // first one still moves when the OS preference does.
        assert_eq!(
            presentation_script(&ViewscreenPresentation::following_system()),
            "window.PhoenixViewscreenPresentation = {\"textScale\":null,\"contrast\":null};"
        );
    }

    #[test]
    fn an_out_of_range_size_is_clamped_before_it_reaches_the_page() {
        let script = presentation_script(&ViewscreenPresentation {
            text_scale_percent: Some(900),
            contrast: None,
        });
        assert!(script.contains(&format!("\"textScale\":{SUPPORTED_TEXT_SCALE_MAX}")));
        // …and the floor is guarded in the other direction, where there is no
        // exposed control at all: only a hand-edited file can ask for it.
        let tiny = presentation_script(&ViewscreenPresentation {
            text_scale_percent: Some(10),
            contrast: None,
        });
        assert!(tiny.contains(&format!("\"textScale\":{SUPPORTED_TEXT_SCALE_MIN}")));
    }
}
