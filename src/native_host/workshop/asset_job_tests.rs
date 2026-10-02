use super::*;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

fn temporary() -> PathBuf {
    std::env::temp_dir().join(format!("phoenix-asset-job-{}", uuid::Uuid::new_v4()))
}

#[test]
fn stages_exact_bytes_and_retires_only_its_own_review_routes() {
    let stage = temporary();
    let documents = HostedDocuments::default();
    documents.publish("/unrelated", "retained".into());
    let mut job = AssetJob::stage(
        stage.clone(),
        documents.clone(),
        &BTreeMap::from([("assets/source.toml".into(), b"# exact\r\n".to_vec())]),
    )
    .unwrap();
    assert_eq!(
        fs::read(stage.join("assets/source.toml")).unwrap(),
        b"# exact\r\n"
    );
    job.publish("/review/0".into(), vec![1, 2], "application/octet-stream");
    assert!(documents.resource("/review/0").unwrap().immutable);
    job.retire();
    assert!(!stage.exists());
    assert!(documents.resource("/review/0").is_none());
    assert_eq!(documents.get("/unrelated").as_deref(), Some("retained"));
    // Dropping a retired owner cannot remove a later job at the same path.
    fs::create_dir_all(&stage).unwrap();
    drop(job);
    assert!(stage.exists());
    remove_stage(&stage);
}

#[test]
fn partial_staging_failure_removes_the_stage() {
    let stage = temporary();
    let files = BTreeMap::from([
        ("assets".into(), b"a file blocks the directory".to_vec()),
        ("assets/model.glb".into(), vec![1]),
    ]);
    assert!(AssetJob::stage(stage.clone(), HostedDocuments::default(), &files).is_err());
    assert!(!stage.exists());
}

#[test]
fn failed_launch_and_partial_review_are_cleaned_on_drop() {
    let stage = temporary();
    let documents = HostedDocuments::default();
    {
        let mut job = AssetJob::stage(stage.clone(), documents.clone(), &Files::new()).unwrap();
        assert!(job
            .launch(&mut Command::new(stage.join("missing-tool")))
            .is_err());
        job.publish("/partial/0".into(), vec![1], "image/png");
        // The adapter can refuse a later output after publishing this one.
    }
    assert!(!stage.exists());
    assert!(documents.is_empty());
}

#[test]
fn polls_completion_and_captured_failure_details() {
    let stage = temporary();
    let mut job =
        AssetJob::stage(stage.clone(), HostedDocuments::default(), &Files::new()).unwrap();
    let mut command = Command::new("node");
    command
        .args([
            "-e",
            "process.stderr.write('tool refusal');process.exitCode=7",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    job.launch(&mut command).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let output = loop {
        if let Some(output) = job.poll().unwrap() {
            break output;
        }
        assert!(Instant::now() < deadline, "tool never completed");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(output.stderr, b"tool refusal");
    drop(job);
    assert!(!stage.exists());
}

#[test]
fn cancellation_and_drop_stop_descendants_before_retiring_resources() {
    for explicit_cancel in [true, false] {
        let root = temporary();
        let stage = root.join("stage");
        let marker = root.join("descendant.txt");
        let documents = HostedDocuments::default();
        let mut job = AssetJob::stage(stage.clone(), documents.clone(), &Files::new()).unwrap();
        let descendant =
            "setInterval(()=>require('fs').appendFileSync(process.env.PHOENIX_TEST_MARKER,'x'),20)";
        let parent = format!("require('child_process').spawn(process.execPath,['-e',{}],{{stdio:'ignore',env:process.env}});setInterval(()=>{{}},1000)", serde_json::to_string(descendant).unwrap());
        let mut command = Command::new("node");
        command
            .args(["-e", &parent])
            .env("PHOENIX_TEST_MARKER", &marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        job.launch(&mut command).unwrap();
        job.publish("/active/0".into(), vec![1], "image/png");
        let deadline = Instant::now() + Duration::from_secs(5);
        while fs::metadata(&marker).map(|value| value.len()).unwrap_or(0) == 0
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        if explicit_cancel {
            job.retire();
        }
        drop(job);
        let before = fs::metadata(&marker).map(|value| value.len()).unwrap_or(0);
        assert!(before > 0, "descendant never started");
        assert!(!stage.exists());
        assert!(documents.is_empty());
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(
            before,
            fs::metadata(&marker).unwrap().len(),
            "descendant survived retirement"
        );
        remove_stage(&root);
    }
}
