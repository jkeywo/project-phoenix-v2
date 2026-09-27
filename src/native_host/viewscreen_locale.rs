//! The native Viewscreen's language choice, independent of Station profiles.

use std::path::PathBuf;

const FILE_NAME: &str = "viewscreen-locale.toml";

#[derive(Clone)]
pub struct ViewscreenLocaleStore {
    dir: PathBuf,
}

impl ViewscreenLocaleStore {
    pub fn user() -> Option<Self> {
        let base = directories::BaseDirs::new()?;
        Some(Self {
            dir: base.data_dir().join("ProjectPhoenix"),
        })
    }

    #[cfg(test)]
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    pub fn load(&self) -> Option<String> {
        let text = std::fs::read_to_string(self.path()).ok()?;
        let saved: SavedLocale = toml::from_str(&text).ok()?;
        valid_locale(&saved.locale).then_some(saved.locale)
    }

    pub fn save(&self, locale: &str) -> std::io::Result<()> {
        if !valid_locale(locale) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid locale",
            ));
        }
        std::fs::create_dir_all(&self.dir)?;
        let text = toml::to_string(&SavedLocale {
            locale: locale.to_owned(),
        })
        .map_err(std::io::Error::other)?;
        super::layout_store::write_atomically(&self.path(), &text)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedLocale {
    locale: String,
}

pub fn valid_locale(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 35
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

pub fn locale_script(saved: Option<&str>) -> String {
    match saved.filter(|value| valid_locale(value)) {
        Some(locale) => format!("window.PhoenixViewscreenLocale = '{locale}';"),
        None => "window.PhoenixViewscreenLocale = null;".to_owned(),
    }
}

pub fn locale_script_value(saved: Option<&str>) -> String {
    match saved.filter(|value| valid_locale(value)) {
        Some(locale) => format!("'{locale}'"),
        None => "null".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_round_trips_and_rejects_script_content() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("phoenix-locale-{nonce}"));
        let store = ViewscreenLocaleStore::at(&dir);
        assert_eq!(store.load(), None);
        store.save("de-DE").unwrap();
        assert_eq!(store.load().as_deref(), Some("de-DE"));
        assert!(!valid_locale("de'; alert(1)"));
        assert!(store.save("de'; alert(1)").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
