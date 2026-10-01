//! Private, per-user reconnect capabilities for native fleet Game Masters.

use std::path::PathBuf;

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

const APP_DIR: &str = "ProjectPhoenix";
const IDENTITIES_DIR: &str = "fleet-identities";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeFleetIdentity {
    pub role: String,
    pub operator_id: Option<String>,
    pub reconnect_credential: String,
    pub role_preset: Option<String>,
    #[serde(default)]
    pub claim: Option<String>,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct NativeFleetIdentityStore {
    root: Option<PathBuf>,
}

impl NativeFleetIdentityStore {
    pub fn user() -> Self {
        let override_dir = std::env::var_os("PHOENIX_FLEET_IDENTITY_DIR");
        let root = identity_root(
            override_dir.clone(),
            directories::BaseDirs::new()
                .map(|base| base.data_dir().join(APP_DIR).join(IDENTITIES_DIR)),
        );
        if override_dir.is_some() && root.is_none() {
            eprintln!("phoenix-host: PHOENIX_FLEET_IDENTITY_DIR must be absolute; reconnect storage is disabled");
        }
        Self { root }
    }

    #[cfg(test)]
    pub fn at(root: PathBuf) -> Self {
        Self { root: Some(root) }
    }

    pub fn load(&self, canonical_code: &str) -> Result<Option<NativeFleetIdentity>, String> {
        let Some(path) = self.path(canonical_code) else {
            return Ok(None);
        };
        match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw)
                .map(Some)
                .map_err(|error| format!("invalid stored native fleet identity: {error}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("cannot read native fleet identity: {error}")),
        }
    }

    pub fn save(&self, canonical_code: &str, identity: &NativeFleetIdentity) -> Result<(), String> {
        let Some(path) = self.path(canonical_code) else {
            return Err("native app-data directory is unavailable".into());
        };
        let raw = serde_json::to_string(identity)
            .map_err(|error| format!("cannot encode native fleet identity: {error}"))?;
        crate::native_file::write_preferences(&path, &raw)
            .map_err(|error| format!("cannot save native fleet identity: {error}"))
    }

    fn path(&self, canonical_code: &str) -> Option<PathBuf> {
        self.root
            .as_ref()
            .map(|root| root.join(format!("{:016x}.json", code_hash(canonical_code))))
    }
}

// Windows Known Folders ignores APPDATA changes. A bounded local matrix needs
// an explicit root per process so a second GM does not reclaim the first's
// saved capability. An invalid override fails closed instead of touching the
// ordinary user's store. The normal application path remains unchanged.
fn identity_root(
    override_dir: Option<std::ffi::OsString>,
    default: Option<PathBuf>,
) -> Option<PathBuf> {
    match override_dir {
        Some(path) => {
            let path = PathBuf::from(path);
            path.is_absolute().then_some(path)
        }
        None => default,
    }
}

/// Stable FNV-1a rather than `DefaultHasher`, whose algorithm is not a storage
/// format. The code is an index, not the capability; the capability remains in
/// the private file contents and is never logged.
fn code_hash(code: &str) -> u64 {
    code.as_bytes()
        .iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // UUID isolates a disposable temp fixture.
mod tests {
    use super::*;

    #[test]
    fn matrix_identity_override_is_absolute_and_never_falls_back_to_user_storage() {
        let default = std::env::temp_dir().join("ordinary-user-identities");
        let isolated = std::env::temp_dir().join("isolated-native-peer");
        assert_eq!(
            identity_root(None, Some(default.clone())),
            Some(default.clone())
        );
        assert_eq!(
            identity_root(
                Some(isolated.clone().into_os_string()),
                Some(default.clone())
            ),
            Some(isolated)
        );
        for invalid in ["", "relative/identities"] {
            assert_eq!(
                identity_root(Some(invalid.into()), Some(default.clone())),
                None
            );
        }
    }

    #[test]
    fn identity_round_trips_under_a_hashed_name() {
        let root = std::env::temp_dir().join(format!(
            "phoenix-native-fleet-identity-{}",
            uuid::Uuid::new_v4()
        ));
        let store = NativeFleetIdentityStore::at(root.clone());
        let identity = NativeFleetIdentity {
            role: "gm".into(),
            operator_id: Some("gm-1".into()),
            reconnect_credential: "private-capability".into(),
            role_preset: None,
            claim: Some("slot-2".into()),
        };
        store.save("ABCD-EFGH", &identity).unwrap();
        assert_eq!(store.load("ABCD-EFGH").unwrap(), Some(identity));
        let names = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 1);
        assert!(!names[0].contains("ABCD"));
        let _ = std::fs::remove_dir_all(root);
    }
}
