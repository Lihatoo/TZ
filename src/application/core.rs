use std::{
    fs::{self, Metadata, Permissions},
    io::{self, Read},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::domain::{
    ActiveConfig, CoreDescriptor, CoreManifest, load_import_manifest, load_manifest,
};
use crate::platform::{AppLock, AppPaths, atomic_write, ensure_not_running};

const VERSION_TIMEOUT: Duration = Duration::from_secs(2);
const VERSION_OUTPUT_LIMIT: u64 = 64 * 1024;
static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct CoreInfo {
    pub descriptor: CoreDescriptor,
    pub version_output: Option<String>,
}

pub fn add(paths: &AppPaths, source: &Path) -> Result<CoreInfo, io::Error> {
    let source_metadata = fs::symlink_metadata(source)?;
    require_directory(source, &source_metadata)?;
    let source = fs::canonicalize(source)?;

    fs::create_dir_all(paths.cores_dir())?;
    let cores_dir = fs::canonicalize(paths.cores_dir())?;
    if source.starts_with(&cores_dir) {
        return Err(invalid_input(format!(
            "core source {} must not be inside {}",
            source.display(),
            cores_dir.display()
        )));
    }

    validate_tree(&source)?;
    let manifest = load_import_manifest(&source)?;
    let target = cores_dir.join(&manifest.core.name);
    let _lock = AppLock::acquire(&paths.lock_file())?;
    match fs::symlink_metadata(&target) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("core `{}` already exists", manifest.core.name),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let version_output = run_version_command(&source, &manifest)?;
    if let Some(output) = version_output.as_deref()
        && !output.contains(&manifest.core.version)
    {
        return Err(invalid_input(format!(
            "core version output does not contain manifest version `{}`",
            manifest.core.version
        )));
    }
    let staging = cores_dir.join(format!(
        ".staging-{}-{}",
        manifest.core.name,
        unique_suffix()
    ));
    let cleanup = StagingGuard(staging.clone());
    copy_tree(&source, &staging)?;
    validate_tree(&staging)?;
    load_import_manifest(&staging)?;
    fs::rename(&staging, &target)?;
    std::mem::forget(cleanup);

    let descriptor = CoreDescriptor {
        name: manifest.core.name.clone(),
        dir: target,
        manifest: load_manifest(&cores_dir.join(&manifest.core.name))?,
    };
    Ok(CoreInfo {
        descriptor,
        version_output,
    })
}

pub fn info(paths: &AppPaths, name: Option<&str>) -> Result<CoreInfo, io::Error> {
    let active = ActiveConfig::load(&paths.active_file())?;
    let name = match name {
        Some(name) => name,
        None if !active.current.core.is_empty() => &active.current.core,
        None => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no active core is selected",
            ));
        }
    };
    let dir = paths.cores_dir().join(valid_core_name(name)?);
    let manifest = load_manifest(&dir).map_err(|error| map_missing_core(name, error))?;
    let version_output = run_version_command(&dir, &manifest)?;
    Ok(CoreInfo {
        descriptor: CoreDescriptor {
            name: name.to_owned(),
            dir,
            manifest,
        },
        version_output,
    })
}

pub fn use_core(paths: &AppPaths, name: &str) -> Result<(), io::Error> {
    let _lock = AppLock::acquire(&paths.lock_file())?;
    ensure_not_running(&paths.core_pid_file())?;
    let name = valid_core_name(name)?;
    load_manifest(&paths.cores_dir().join(name)).map_err(|error| map_missing_core(name, error))?;

    let mut active = ActiveConfig::load(&paths.active_file())?;
    active.current.core = name.to_owned();
    write_active(paths, &active)
}

pub fn remove(paths: &AppPaths, name: &str) -> Result<(), io::Error> {
    let _lock = AppLock::acquire(&paths.lock_file())?;
    ensure_not_running(&paths.core_pid_file())?;
    let name = valid_core_name(name)?;

    let target = paths.cores_dir().join(name);
    load_manifest(&target).map_err(|error| map_missing_core(name, error))?;
    let mut active = ActiveConfig::load(&paths.active_file())?;
    let tombstone = paths
        .cores_dir()
        .join(format!(".removing-{name}-{}", unique_suffix()));
    fs::rename(&target, &tombstone)?;

    if active.current.core == name {
        active.current.core.clear();
        if let Err(error) = write_active(paths, &active) {
            let _ = fs::rename(&tombstone, &target);
            return Err(error);
        }
    }

    fs::remove_dir_all(&tombstone)?;
    remove_tree_if_present(&paths.generated_dir().join(name))?;
    remove_tree_if_present(&paths.core_workdir(name))?;
    Ok(())
}

fn write_active(paths: &AppPaths, active: &ActiveConfig) -> Result<(), io::Error> {
    let content = toml::to_string_pretty(active)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    atomic_write(&paths.active_file(), content.as_bytes())
}

fn validate_tree(root: &Path) -> Result<(), io::Error> {
    walk_tree(root, &mut |path, metadata| {
        if metadata.file_type().is_symlink() {
            return Err(invalid_input(format!(
                "core package contains symlink: {}",
                path.display()
            )));
        }
        if !metadata.is_dir() && !metadata.is_file() {
            return Err(invalid_input(format!(
                "core package contains special file: {}",
                path.display()
            )));
        }
        Ok(())
    })
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), io::Error> {
    let metadata = fs::symlink_metadata(source)?;
    require_directory(source, &metadata)?;
    fs::create_dir(destination)?;
    fs::set_permissions(destination, cloned_permissions(&metadata))?;

    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            return Err(invalid_input(format!(
                "core package contains symlink: {}",
                source_path.display()
            )));
        } else if metadata.is_dir() {
            copy_tree(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &destination_path)?;
            fs::set_permissions(&destination_path, cloned_permissions(&metadata))?;
        } else {
            return Err(invalid_input(format!(
                "core package contains special file: {}",
                source_path.display()
            )));
        }
    }
    Ok(())
}

fn walk_tree(
    root: &Path,
    visit: &mut impl FnMut(&Path, &Metadata) -> Result<(), io::Error>,
) -> Result<(), io::Error> {
    let metadata = fs::symlink_metadata(root)?;
    visit(root, &metadata)?;
    if metadata.is_dir() {
        for entry in fs::read_dir(root)? {
            walk_tree(&entry?.path(), visit)?;
        }
    }
    Ok(())
}

fn run_version_command(dir: &Path, manifest: &CoreManifest) -> Result<Option<String>, io::Error> {
    let Some(version) = &manifest.commands.version else {
        return Ok(None);
    };
    let args = CoreDescriptor {
        name: manifest.core.name.clone(),
        dir: dir.to_owned(),
        manifest: manifest.clone(),
    }
    .render_args(&version.args, Path::new(""), dir);
    let mut child = Command::new(dir.join(&manifest.core.binary))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout_reader = thread::spawn(move || read_limited(stdout));
    let stderr_reader = thread::spawn(move || read_limited(stderr));

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= VERSION_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("core version command exceeded {VERSION_TIMEOUT:?}"),
            ));
        }
        thread::sleep(Duration::from_millis(10));
    };
    let mut bytes = stdout_reader
        .join()
        .map_err(|_| io::Error::other("core version stdout reader panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| io::Error::other("core version stderr reader panicked"))??;
    if !status.success() {
        return Err(io::Error::other(format!(
            "core version command exited with {status}"
        )));
    }
    if bytes.len() < VERSION_OUTPUT_LIMIT as usize {
        bytes.extend_from_slice(
            &stderr[..stderr
                .len()
                .min(VERSION_OUTPUT_LIMIT as usize - bytes.len())],
        );
    }
    Ok(Some(String::from_utf8_lossy(&bytes).trim().to_owned()))
}

fn read_limited(mut input: impl Read) -> Result<Vec<u8>, io::Error> {
    let mut kept = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = VERSION_OUTPUT_LIMIT as usize - kept.len();
        kept.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    Ok(kept)
}

fn require_directory(path: &Path, metadata: &Metadata) -> Result<(), io::Error> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid_input(format!(
            "core source must be a regular directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn cloned_permissions(metadata: &Metadata) -> Permissions {
    Permissions::from_mode(metadata.permissions().mode())
}

fn remove_tree_if_present(path: &Path) -> Result<(), io::Error> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(path)
        }
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn valid_core_name(name: &str) -> Result<&str, io::Error> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(invalid_input(format!(
            "invalid core name `{name}`; expected ASCII letters, numbers, '.', '_' or '-'"
        )));
    }
    Ok(name)
}

fn map_missing_core(name: &str, error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::NotFound {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("core `{name}` does not exist"),
        )
    } else {
        error
    }
}

fn invalid_input(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{nanos}-{counter}", std::process::id())
}

struct StagingGuard(PathBuf);

impl Drop for StagingGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::fs::symlink, os::unix::net::UnixListener};
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, AppPaths) {
        let root = tempdir().unwrap();
        let paths = AppPaths::unified(root.path());
        paths.ensure_dirs().unwrap();
        ActiveConfig::default().save(&paths.active_file()).unwrap();
        (root, paths)
    }

    fn source_core(root: &Path, source_name: &str, name: &str) -> PathBuf {
        let dir = root.join(source_name);
        fs::create_dir(&dir).unwrap();
        let manifest = format!(
            r#"schema_version = 1
[core]
name = "{name}"
family = "clash"
version = "1.0.0"
binary = "mihomo"
os = "{}"
arch = "{}"
[runtime]
entrypoint = "config.yaml"
format = "yaml"
[capabilities.config]
mixed_proxy = true
http_proxy = true
socks_proxy = true
api = true
dns = true
tun = true
[commands.start]
args = ["-f", "{{config}}"]
[commands.version]
args = ["--version"]
"#,
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        fs::write(dir.join("core.toml"), manifest).unwrap();
        fs::write(dir.join("mihomo"), "#!/bin/sh\necho version-1.0.0\n").unwrap();
        fs::set_permissions(dir.join("mihomo"), Permissions::from_mode(0o755)).unwrap();
        dir
    }

    #[test]
    fn add_imports_from_differently_named_source_via_staging() {
        let (_root, paths) = setup();
        let sources = tempdir().unwrap();
        let source = source_core(sources.path(), "downloaded-core", "mihomo");
        let added = add(&paths, &source).unwrap();
        assert_eq!(added.descriptor.name, "mihomo");
        assert_eq!(added.version_output.as_deref(), Some("version-1.0.0"));
        assert!(paths.cores_dir().join("mihomo/core.toml").is_file());
        assert!(fs::read_dir(paths.cores_dir()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".staging-")
        }));
    }

    #[test]
    fn add_rejects_version_mismatch_without_creating_target() {
        let (_root, paths) = setup();
        let sources = tempdir().unwrap();
        let source = source_core(sources.path(), "source", "mihomo");
        fs::write(source.join("mihomo"), "#!/bin/sh\necho version-2.0.0\n").unwrap();
        fs::set_permissions(source.join("mihomo"), Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            add(&paths, &source).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(!paths.cores_dir().join("mihomo").exists());
    }

    #[test]
    fn add_rejects_symlinks_and_existing_target() {
        let (_root, paths) = setup();
        let sources = tempdir().unwrap();
        let source = source_core(sources.path(), "source", "mihomo");
        symlink(source.join("core.toml"), source.join("linked.toml")).unwrap();
        assert_eq!(
            add(&paths, &source).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        fs::remove_file(source.join("linked.toml")).unwrap();
        add(&paths, &source).unwrap();
        assert_eq!(
            add(&paths, &source).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
    }

    #[test]
    fn add_rejects_special_files() {
        let (_root, paths) = setup();
        let sources = tempdir().unwrap();
        let source = source_core(sources.path(), "source", "mihomo");
        let _socket = UnixListener::bind(source.join("runtime.sock")).unwrap();
        assert_eq!(
            add(&paths, &source).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(!paths.cores_dir().join("mihomo").exists());
    }

    #[test]
    fn use_and_remove_reject_a_running_managed_process() {
        let (_root, paths) = setup();
        let sources = tempdir().unwrap();
        let source = source_core(sources.path(), "source", "mihomo");
        add(&paths, &source).unwrap();
        fs::write(paths.core_pid_file(), std::process::id().to_string()).unwrap();
        assert_eq!(
            use_core(&paths, "mihomo").unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(
            remove(&paths, "mihomo").unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(paths.cores_dir().join("mihomo").is_dir());
    }

    #[test]
    fn remove_current_clears_active_and_derived_state() {
        let (_root, paths) = setup();
        let sources = tempdir().unwrap();
        let source = source_core(sources.path(), "source", "mihomo");
        add(&paths, &source).unwrap();
        use_core(&paths, "mihomo").unwrap();
        fs::create_dir_all(paths.generated_dir().join("mihomo")).unwrap();
        fs::create_dir_all(paths.core_workdir("mihomo")).unwrap();
        fs::write(paths.generated_dir().join("mihomo/config.yaml"), "old").unwrap();
        remove(&paths, "mihomo").unwrap();
        assert!(
            ActiveConfig::load(&paths.active_file())
                .unwrap()
                .current
                .core
                .is_empty()
        );
        assert!(!paths.cores_dir().join("mihomo").exists());
        assert!(!paths.generated_dir().join("mihomo").exists());
        assert!(!paths.core_workdir("mihomo").exists());
    }

    #[test]
    fn version_command_times_out_without_a_shell() {
        let (_root, paths) = setup();
        let sources = tempdir().unwrap();
        let source = source_core(sources.path(), "source", "mihomo");
        fs::write(source.join("mihomo"), "#!/bin/sh\nexec sleep 5\n").unwrap();
        fs::set_permissions(source.join("mihomo"), Permissions::from_mode(0o755)).unwrap();
        let started = Instant::now();
        assert_eq!(
            add(&paths, &source).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn version_arguments_are_not_shell_interpreted() {
        let (_root, paths) = setup();
        let sources = tempdir().unwrap();
        let source = source_core(sources.path(), "source", "mihomo");
        let marker = sources.path().join("should-not-exist");
        let manifest_file = source.join("core.toml");
        let body = fs::read_to_string(&manifest_file).unwrap().replace(
            "args = [\"--version\"]",
            &format!("args = [\";touch {}\"]", marker.display()),
        );
        fs::write(manifest_file, body).unwrap();
        let result = add(&paths, &source).unwrap();
        assert!(result.version_output.unwrap().contains("version-1.0.0"));
        assert!(!marker.exists());
    }

    #[test]
    fn rejects_unsafe_lookup_names() {
        let (_root, paths) = setup();
        assert_eq!(
            use_core(&paths, "../outside").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            remove(&paths, "../outside").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            info(&paths, Some("../outside")).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn info_defaults_to_active_core() {
        let (_root, paths) = setup();
        let sources = tempdir().unwrap();
        let source = source_core(sources.path(), "source", "mihomo");
        add(&paths, &source).unwrap();
        use_core(&paths, "mihomo").unwrap();
        assert_eq!(info(&paths, None).unwrap().descriptor.name, "mihomo");
    }
}
