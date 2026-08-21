use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedProcess {
    NotRunning,
    Running(i32),
    Stale(i32),
}

pub fn managed_process(pid_file: &Path) -> Result<ManagedProcess, io::Error> {
    let Some(pid) = read_pid(pid_file)? else {
        return Ok(ManagedProcess::NotRunning);
    };
    if is_process_alive(pid) {
        Ok(ManagedProcess::Running(pid))
    } else {
        Ok(ManagedProcess::Stale(pid))
    }
}

pub fn ensure_not_running(pid_file: &Path) -> Result<(), io::Error> {
    match managed_process(pid_file)? {
        ManagedProcess::Running(pid) => Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            format!("managed core is running with pid {pid}"),
        )),
        ManagedProcess::NotRunning | ManagedProcess::Stale(_) => Ok(()),
    }
}

pub fn read_pid(file: &Path) -> Result<Option<i32>, io::Error> {
    if !file.is_file() {
        return Ok(None);
    }
    let content = fs::read_to_string(file)?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let pid = trimmed
        .parse::<i32>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    if pid <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid managed PID `{pid}` in {}", file.display()),
        ));
    }
    Ok(Some(pid))
}

pub fn is_process_alive(pid: i32) -> bool {
    if unsafe { libc_kill(pid, 0) } != 0 {
        return false;
    }
    !matches!(process_state(pid), Ok('Z'))
}

pub fn process_executable(pid: i32) -> Result<PathBuf, io::Error> {
    fs::read_link(format!("/proc/{pid}/exe"))
}

pub fn ensure_owned_process(pid: i32, expected_binary: &Path) -> Result<(), io::Error> {
    let process_dir = PathBuf::from(format!("/proc/{pid}"));
    let owner = fs::metadata(&process_dir)?.uid();
    if owner != effective_uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("pid {pid} 不属于当前用户，拒绝停止"),
        ));
    }
    let actual = process_executable(pid)?;
    let expected = fs::canonicalize(expected_binary)?;
    if actual != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "pid {pid} 不是受管 core（实际 {}，预期 {}）",
                actual.display(),
                expected.display()
            ),
        ));
    }
    Ok(())
}

pub fn terminate_process(pid: i32, force: bool) -> Result<(), io::Error> {
    let signal = if force { 9 } else { 15 };
    if unsafe { libc_kill(pid, signal) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn process_state(pid: i32) -> Result<char, io::Error> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let fields = stat
        .rsplit_once(')')
        .map(|(_, fields)| fields.trim_start())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid /proc stat"))?;
    fields
        .chars()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing process state"))
}

unsafe fn libc_kill(pid: i32, sig: i32) -> i32 {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    unsafe { kill(pid, sig) }
}

fn effective_uid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}

#[cfg(test)]
mod tests {
    use super::{ManagedProcess, managed_process, read_pid};
    use tempfile::tempdir;

    #[test]
    fn rejects_non_positive_pid() {
        let root = tempdir().unwrap();
        let file = root.path().join("core.pid");
        std::fs::write(&file, "-1\n").unwrap();
        assert!(read_pid(&file).is_err());
    }

    #[test]
    fn detects_current_process() {
        let root = tempdir().unwrap();
        let file = root.path().join("core.pid");
        std::fs::write(&file, std::process::id().to_string()).unwrap();
        assert_eq!(
            managed_process(&file).unwrap(),
            ManagedProcess::Running(std::process::id() as i32)
        );
    }
}
