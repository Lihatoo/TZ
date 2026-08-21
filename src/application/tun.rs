use std::{fs, io, process::Command};

use crate::{
    domain::{ActiveConfig, load_manifest},
    platform::{AppLock, AppPaths, ManagedProcess, managed_process},
};

pub fn status(paths: &AppPaths) -> Result<(), io::Error> {
    let active = ActiveConfig::load(&paths.active_file())?;
    let (supported, privileged) = tun_capability(paths).unwrap_or((false, false));
    println!(
        "tun={} supported={} permission={}",
        on_off(active.tun.enabled),
        yes_no(supported),
        if privileged { "ready" } else { "missing" }
    );
    Ok(())
}

pub fn set(paths: &AppPaths, enabled: bool) -> Result<(), io::Error> {
    if enabled {
        validate_tun(paths)?;
    }
    let original = ActiveConfig::load(&paths.active_file())?;
    if original.tun.enabled == enabled {
        println!("tun 已经是 {}", on_off(enabled));
        return Ok(());
    }
    let was_running = matches!(
        managed_process(&paths.core_pid_file())?,
        ManagedProcess::Running(_)
    );
    save_enabled(paths, enabled)?;
    if was_running && let Err(error) = super::service::restart(paths) {
        let rollback =
            save_enabled(paths, original.tun.enabled).and_then(|_| super::service::start(paths));
        return Err(io::Error::other(match rollback {
            Ok(()) => format!("TUN 切换失败，已恢复原运行状态: {error}"),
            Err(rollback_error) => {
                format!("TUN 切换失败且恢复失败: {error}; {rollback_error}")
            }
        }));
    }
    println!("tun {}", on_off(enabled));
    Ok(())
}

fn validate_tun(paths: &AppPaths) -> Result<(), io::Error> {
    let active = ActiveConfig::load(&paths.active_file())?;
    if active.current.core.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "未选择 core"));
    }
    let core_dir = paths.cores_dir().join(&active.current.core);
    let manifest = load_manifest(&core_dir)?;
    if !manifest.capabilities.config.tun {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("core `{}` 不支持 TUN", active.current.core),
        ));
    }
    if !std::path::Path::new("/dev/net/tun").exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "系统不存在 /dev/net/tun",
        ));
    }
    let binary = core_dir.join(&manifest.core.binary);
    if !has_net_admin(&binary) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "TUN 需要 CAP_NET_ADMIN；请执行 `sudo setcap cap_net_admin,cap_net_raw+ep '{}'` 后重试",
                shell_path(&binary)
            ),
        ));
    }
    Ok(())
}

fn tun_capability(paths: &AppPaths) -> Result<(bool, bool), io::Error> {
    let active = ActiveConfig::load(&paths.active_file())?;
    if active.current.core.is_empty() {
        return Ok((false, false));
    }
    let core_dir = paths.cores_dir().join(&active.current.core);
    let manifest = load_manifest(&core_dir)?;
    let supported = manifest.capabilities.config.tun;
    let privileged = std::path::Path::new("/dev/net/tun").exists()
        && has_net_admin(&core_dir.join(&manifest.core.binary));
    Ok((supported, privileged))
}

fn has_net_admin(binary: &std::path::Path) -> bool {
    if effective_uid() == 0 {
        return true;
    }
    Command::new("getcap")
        .arg(binary)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| String::from_utf8_lossy(&output.stdout).contains("cap_net_admin"))
}

fn save_enabled(paths: &AppPaths, enabled: bool) -> Result<(), io::Error> {
    let _lock = AppLock::acquire(&paths.lock_file())?;
    let mut active = ActiveConfig::load(&paths.active_file())?;
    active.tun.enabled = enabled;
    if !active.current.core.is_empty() {
        let generated = paths.generated_dir().join(&active.current.core);
        match fs::remove_dir_all(generated) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    active.save(&paths.active_file())
}

fn effective_uid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn shell_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\'', "'\"'\"'")
}
