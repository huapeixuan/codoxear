use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct VoiceOwnerLock {
    path: PathBuf,
}

impl VoiceOwnerLock {
    pub fn acquire(app_dir: &Path, role: &str) -> Result<Self, String> {
        fs::create_dir_all(app_dir)
            .map_err(|err| format!("create {}: {err}", app_dir.display()))?;
        let path = app_dir.join("voice_worker.lock");
        let mut file = create_new_lock_file(&path).map_err(|err| {
            if err.kind() == std::io::ErrorKind::AlreadyExists {
                format!("voice owner lock already exists: {}", path.display())
            } else {
                format!("create {}: {err}", path.display())
            }
        })?;
        let now = crate::voice_state::now_seconds();
        let body = format!(
            "pid={}\nrole={}\ntimestamp={now:.6}\nprocess=codoxear-backend-rs\n",
            std::process::id(),
            role.trim()
        );
        file.write_all(body.as_bytes())
            .map_err(|err| format!("write {}: {err}", path.display()))?;
        file.sync_all()
            .map_err(|err| format!("fsync {}: {err}", path.display()))?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for VoiceOwnerLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn create_new_lock_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}
