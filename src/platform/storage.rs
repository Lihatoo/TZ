use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions, Permissions},
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
};
use tempfile::NamedTempFile;

#[derive(Debug)]
pub struct AppLock {
    file: File,
}

impl AppLock {
    pub fn acquire(path: &Path) -> Result<Self, io::Error> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        file.try_lock_exclusive().map_err(|error| {
            if error.kind() == io::ErrorKind::WouldBlock {
                io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!("another tz operation holds {}", path.display()),
                )
            } else {
                error
            }
        })?;
        file.set_len(0)?;
        writeln!(file, "{}", std::process::id())?;
        file.sync_data()?;
        Ok(Self { file })
    }
}

impl Drop for AppLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub fn atomic_write(path: &Path, content: &[u8]) -> Result<(), io::Error> {
    atomic_write_with_mode(path, content, 0o644)
}

pub fn atomic_write_private(path: &Path, content: &[u8]) -> Result<(), io::Error> {
    atomic_write_with_mode(path, content, 0o600)
}

fn atomic_write_with_mode(path: &Path, content: &[u8], mode: u32) -> Result<(), io::Error> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path has no parent: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent)?;

    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary
        .as_file()
        .set_permissions(Permissions::from_mode(mode))?;
    temporary.write_all(content)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;

    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AppLock, atomic_write, atomic_write_private};
    use std::{fs, os::unix::fs::PermissionsExt};
    use tempfile::tempdir;

    #[test]
    fn atomically_replaces_existing_content() {
        let root = tempdir().unwrap();
        let file = root.path().join("config.toml");
        atomic_write(&file, b"old").unwrap();
        atomic_write(&file, b"new").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"new");
    }

    #[test]
    fn private_write_uses_user_only_permissions() {
        let root = tempdir().unwrap();
        let file = root.path().join("profiles.toml");
        atomic_write_private(&file, b"secret").unwrap();
        assert_eq!(
            fs::metadata(file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn second_lock_is_rejected_until_drop() {
        let root = tempdir().unwrap();
        let path = root.path().join("tz.lock");
        let lock = AppLock::acquire(&path).unwrap();
        assert_eq!(
            AppLock::acquire(&path).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        drop(lock);
        AppLock::acquire(&path).unwrap();
    }
}
