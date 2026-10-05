use super::*;
#[test]
#[allow(clippy::disallowed_methods)]
fn both_policies_replace_bytes_and_clean_failed_replacements() {
    let root = std::env::temp_dir().join(format!("phoenix-file-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    for policy in [
        TemporaryPolicy::Preference,
        TemporaryPolicy::Unique {
            prefix: ".phoenix-workshop-",
        },
    ] {
        let target = root.join("saved");
        replace(&target, b"old", TemporaryPolicy::Preference).unwrap();
        replace(&target, &[0, 255, 1], policy).unwrap();
        assert_eq!(fs::read(&target).unwrap(), [0, 255, 1]);
        fs::remove_file(&target).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), b"original").unwrap();
        assert!(replace(&target, b"new", TemporaryPolicy::Preference).is_err());
        assert!(replace(
            &target,
            b"new",
            TemporaryPolicy::Unique {
                prefix: ".phoenix-workshop-"
            }
        )
        .is_err());
        assert_eq!(fs::read(target.join("keep")).unwrap(), b"original");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(&target).unwrap();
    }
    let target = root.join("saved");
    fs::write(&target, b"old").unwrap();
    let temporary = TemporaryPolicy::Preference.path(&target, &root);
    fs::create_dir(&temporary).unwrap();
    assert!(write_preferences(&target, "new").is_err());
    assert_eq!(fs::read(&target).unwrap(), b"old");
    assert!(temporary.is_dir());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn only_recognises_owned_preference_debris() {
    assert!(names_a_temporary("layout.toml.123.tmp"));
    for name in [
        "notes.tmp",
        "old.toml.tmp",
        ".123.tmp",
        "file..tmp",
        "file.pid.tmp",
        ".phoenix-workshop-abc.tmp",
    ] {
        assert!(!names_a_temporary(name));
    }
}
