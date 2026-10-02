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
#[path = "asset_job_tests.rs"]
mod tests;
