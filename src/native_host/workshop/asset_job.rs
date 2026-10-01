//! Private process, stage and review-resource lifetime shared by asset tools.
use crate::{delivery::serve::HostedDocuments, workshop::provider::Files};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
};

pub(super) struct AssetJob {
    stage: PathBuf,
    child: Option<Child>,
    documents: HostedDocuments,
    routes: Vec<String>,
    retired: bool,
}

impl AssetJob {
    pub(super) fn stage(
        stage: PathBuf,
        documents: HostedDocuments,
        files: &Files,
    ) -> Result<Self, String> {
        let job = Self {
            stage,
            child: None,
            documents,
            routes: Vec::new(),
            retired: false,
        };
        fs::create_dir_all(&job.stage).map_err(|error| error.to_string())?;
        for (path, bytes) in files {
            let target = job
                .stage
                .join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::write(target, bytes).map_err(|error| error.to_string())?;
        }
        Ok(job)
    }

    pub(super) fn directory(&self) -> &Path {
        &self.stage
    }

    pub(super) fn launch(&mut self, command: &mut Command) -> io::Result<()> {
        if self.child.is_some() || self.retired {
            return Err(io::Error::other("Asset job cannot launch another process"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        self.child = Some(command.spawn()?);
        Ok(())
    }

    pub(super) fn poll(&mut self) -> io::Result<Option<Output>> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| io::Error::other("Asset job has no process"))?;
        if child.try_wait()?.is_none() {
            return Ok(None);
        }
        self.child
            .take()
            .expect("polled child")
            .wait_with_output()
            .map(Some)
    }

    pub(super) fn has_review(&self) -> bool {
        !self.routes.is_empty()
    }

    pub(super) fn publish(&mut self, route: String, bytes: Vec<u8>, content_type: &str) {
        self.documents
            .publish_bytes(route.clone(), bytes, content_type, true);
        self.routes.push(route);
    }

    pub(super) fn retire(&mut self) {
        if self.retired {
            return;
        }
        self.retired = true;
        if let Some(mut child) = self.child.take() {
            terminate_tree(&mut child);
        }
        for route in self.routes.drain(..) {
            self.documents.withdraw(&route);
        }
        remove_stage(&self.stage);
    }
}

impl Drop for AssetJob {
    fn drop(&mut self) {
        self.retire();
    }
}

pub(super) fn remove_stage(stage: &Path) {
    let _ = fs::remove_dir_all(stage);
}

fn terminate_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        // Node is only the coordinator; gltf-transform or Blender may be its
        // current child. Kill the whole process tree before deleting the stage.
        if let Ok(mut killer) = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if !wait_bounded(&mut killer, std::time::Duration::from_secs(2)) {
                let _ = killer.kill();
            }
        }
    }
    #[cfg(unix)]
    {
        if let Ok(mut killer) = Command::new("kill")
            // `--` keeps the negative process-group id from being parsed as
            // another option by the external Unix `kill` command.
            .args(["-KILL", "--", &format!("-{}", child.id())])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if !wait_bounded(&mut killer, std::time::Duration::from_secs(2)) {
                let _ = killer.kill();
            }
        }
    }
    if !wait_bounded(child, std::time::Duration::from_secs(2)) {
        let _ = child.kill();
        wait_bounded(child, std::time::Duration::from_secs(2));
    }
}
fn wait_bounded(child: &mut Child, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Err(_) => return false,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Ok(None) => return false,
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
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
            let descendant = "setInterval(()=>require('fs').appendFileSync(process.env.PHOENIX_TEST_MARKER,'x'),20)";
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
}
