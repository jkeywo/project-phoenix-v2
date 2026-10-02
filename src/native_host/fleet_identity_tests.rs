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
