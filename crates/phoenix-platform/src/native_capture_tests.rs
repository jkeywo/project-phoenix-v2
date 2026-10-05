use super::*;
use std::collections::BTreeSet;

struct Scratch(PathBuf);
impl Scratch {
    #[allow(clippy::disallowed_methods)]
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("phoenix-capture-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn admitted_subtrees_keep_exact_bytes_and_skip_unselected_content() {
    let scratch = Scratch::new();
    for name in ["assets", "ignored"] {
        fs::create_dir(scratch.0.join(name)).unwrap();
    }
    let exact = b"\xef\xbb\xbf# retained\r\n[global]\r\nseed=1\n";
    fs::write(scratch.0.join("assets/world.toml"), exact).unwrap();
    fs::write(scratch.0.join("ignored/private.toml"), b"not admitted").unwrap();
    let mut seen = BTreeSet::new();
    walk(&scratch.0, &mut |entry| {
        let name = relative_name(&entry.path(), &scratch.0)?;
        if entry.file_type().unwrap().is_dir() {
            return Ok((name == "assets").then(|| entry.path()));
        }
        seen.insert(name);
        assert_eq!(
            read_bounded(&entry.path(), exact.len() as u64)?,
            exact.as_slice()
        );
        Ok(None)
    })
    .unwrap();
    assert_eq!(seen, BTreeSet::from(["assets/world.toml".into()]));
}

#[test]
fn callback_refusal_stops_before_descending_or_visiting_a_suffix() {
    let scratch = Scratch::new();
    fs::create_dir(scratch.0.join("child")).unwrap();
    fs::write(scratch.0.join("child/content.toml"), b"child").unwrap();
    let mut calls = 0;
    let result = walk(&scratch.0, &mut |_| {
        calls += 1;
        Err("adapter refusal".into())
    });
    assert_eq!(result, Err("adapter refusal".into()));
    assert_eq!(calls, 1);
}

#[test]
fn bounded_read_retains_one_overflow_byte_for_the_adapter_check() {
    let scratch = Scratch::new();
    let file = scratch.0.join("binary.bin");
    fs::write(&file, [0, 255, 2, 3, 4]).unwrap();
    assert_eq!(read_bounded(&file, 2).unwrap(), [0, 255, 2]);
    assert_eq!(read_bounded(&file, 0).unwrap(), [0]);
    assert_eq!(read_bounded(&file, 5).unwrap(), [0, 255, 2, 3, 4]);
    assert!(read_bounded(&scratch.0.join("missing"), 2).is_err());
    assert!(relative_name(&file, &scratch.0.join("child")).is_err());
}

#[test]
fn accumulation_checks_after_insertion_and_counts_replacements_as_one_member() {
    let mut files = std::collections::BTreeMap::new();
    insert_checked(&mut files, "a".into(), vec![1, 2], 1, 2, "source refusal").unwrap();
    insert_checked(&mut files, "a".into(), vec![3], 1, 2, "source refusal").unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(
        insert_checked(&mut files, "b".into(), vec![4], 1, 2, "source refusal"),
        Err("source refusal".into())
    );
    assert_eq!(files.len(), 2); // The original adapters checked after inserting.
    assert!(check_size(16383, 512, 16383, 512, "dependency refusal").is_ok());
    assert_eq!(
        check_size(16384, 512, 16383, 512, "dependency refusal"),
        Err("dependency refusal".into())
    );
    assert_eq!(
        check_size(1, 3, 1, 2, "byte refusal"),
        Err("byte refusal".into())
    );
}
