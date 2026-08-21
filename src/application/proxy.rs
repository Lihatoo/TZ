use std::{
    env, fs, io,
    process::{Command, Output},
};

use serde::{Deserialize, Serialize};

use crate::{
    domain::{ActiveConfig, RuntimeConfig, Settings, load_manifest},
    platform::{AppLock, AppPaths, ManagedProcess, atomic_write_private, managed_process},
};

#[derive(Debug, Serialize, Deserialize)]
struct SystemProxyBackup {
    mode: String,
    http_host: String,
    http_port: String,
    https_host: String,
    https_port: String,
    socks_host: String,
    socks_port: String,
    ignore_hosts: String,
}

pub fn status(paths: &AppPaths) -> Result<(), io::Error> {
    let active = ActiveConfig::load(&paths.active_file())?;
    let runtime = RuntimeConfig::load(&paths.runtime_file())?;
    let desktop = gsettings(&["get", "org.gnome.system.proxy", "mode"])
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|_| "unavailable".into());
    println!(
        "terminal={} system={} desktop={} mixed=127.0.0.1:{}",
        on_off(active.shell_proxy.enabled),
        on_off(active.system_proxy.enabled),
        desktop,
        runtime.proxy.mixed_port
    );
    Ok(())
}

pub fn terminal(paths: &AppPaths, enabled: bool) -> Result<(), io::Error> {
    update_active(paths, |active| active.shell_proxy.enabled = enabled)?;
    println!("terminal proxy {}", on_off(enabled));
    println!(
        "当前 shell 请执行: eval \"$(tz proxy {})\"",
        if enabled { "env" } else { "noenv" }
    );
    Ok(())
}

pub fn system(paths: &AppPaths, enabled: bool) -> Result<(), io::Error> {
    if enabled
        && !matches!(
            managed_process(&paths.core_pid_file())?,
            ManagedProcess::Running(_)
        )
    {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "服务未运行，拒绝把桌面代理指向未监听端口",
        ));
    }
    let _lock = AppLock::acquire(&paths.lock_file())?;
    let runtime = RuntimeConfig::load(&paths.runtime_file())?;
    let mut active = ActiveConfig::load(&paths.active_file())?;
    let (http_port, socks_port) = proxy_ports(paths, &runtime)?;
    let backup_path = paths.runtime_dir().join("system-proxy-backup.json");
    if !enabled && !active.system_proxy.enabled && !backup_path.is_file() {
        println!("system proxy 已经是 off");
        return Ok(());
    }
    let backup = if enabled {
        if active.system_proxy.enabled {
            load_backup(&backup_path)?
        } else {
            let backup = capture_system_proxy()?;
            let content = serde_json::to_vec_pretty(&backup)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            atomic_write_private(&backup_path, &content)?;
            Some(backup)
        }
    } else {
        load_backup(&backup_path)?
    };
    let applied = if enabled {
        let bypass = bypass_items(paths, active.system_proxy.bypass, true)?;
        let http_port = http_port.to_string();
        let socks_port = socks_port.to_string();
        apply_system_proxy(&http_port, &socks_port, &bypass)
    } else if let Some(backup) = &backup {
        restore_system_proxy(backup)
    } else {
        gsettings(&["set", "org.gnome.system.proxy", "mode", "none"]).map(|_| ())
    };
    if let Err(error) = applied {
        if let Some(backup) = &backup {
            let _ = restore_system_proxy(backup);
        }
        return Err(error);
    }
    active.system_proxy.enabled = enabled;
    if let Err(error) = active.save(&paths.active_file()) {
        if let Some(backup) = &backup {
            let _ = restore_system_proxy(backup);
        }
        return Err(error);
    }
    if !enabled {
        match fs::remove_file(&backup_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    println!("system proxy {}", on_off(enabled));
    Ok(())
}

pub fn both(paths: &AppPaths, enabled: bool) -> Result<(), io::Error> {
    if enabled {
        update_active(paths, |active| active.shell_proxy.enabled = true)?;
        if let Err(error) = system(paths, true) {
            let _ = update_active(paths, |active| active.shell_proxy.enabled = false);
            return Err(error);
        }
    } else {
        system(paths, false)?;
        update_active(paths, |active| active.shell_proxy.enabled = false)?;
    }
    println!("terminal + system proxy {}", on_off(enabled));
    println!(
        "当前 shell 请执行: eval \"$(tz proxy {})\"",
        if enabled { "env" } else { "noenv" }
    );
    Ok(())
}

pub fn env(paths: &AppPaths, shell: &str) -> Result<(), io::Error> {
    let runtime = RuntimeConfig::load(&paths.runtime_file())?;
    let active = ActiveConfig::load(&paths.active_file())?;
    let bypass = bypass_items(paths, active.shell_proxy.bypass, false)?.join(",");
    let (http_port, socks_port) = proxy_ports(paths, &runtime)?;
    print_env(shell, http_port, socks_port, &bypass);
    Ok(())
}

pub fn noenv(shell: &str) -> Result<(), io::Error> {
    match shell {
        "bash" | "zsh" => println!(
            "unset http_proxy https_proxy all_proxy HTTP_PROXY HTTPS_PROXY ALL_PROXY NO_PROXY no_proxy"
        ),
        "fish" => println!(
            "set -e http_proxy https_proxy all_proxy HTTP_PROXY HTTPS_PROXY ALL_PROXY NO_PROXY no_proxy"
        ),
        _ => return Err(invalid_shell(shell)),
    }
    Ok(())
}

pub fn shell_init(paths: &AppPaths, shell: &str) -> Result<(), io::Error> {
    match shell {
        "bash" | "zsh" => print!(
            r#"tz() {{
  command tz "$@"
  local rc=$?
  if [ "$rc" -eq 0 ] && [ "${{1:-}}" = proxy ]; then
    case "${{2:-}}:${{3:-}}" in
      on:|terminal:on) eval "$(command tz proxy env {shell})" ;;
      off:|terminal:off) eval "$(command tz proxy noenv {shell})" ;;
    esac
  fi
  return "$rc"
}}
"#
        ),
        "fish" => print!(
            r#"function tz
  command tz $argv
  set -l rc $status
  if test $rc -eq 0; and test (count $argv) -ge 2; and test $argv[1] = proxy
    if test $argv[2] = on; or test (count $argv) -ge 3; and test $argv[2] = terminal; and test $argv[3] = on
      command tz proxy env fish | source
    else if test $argv[2] = off; or test (count $argv) -ge 3; and test $argv[2] = terminal; and test $argv[3] = off
      command tz proxy noenv fish | source
    end
  end
  return $rc
end
"#
        ),
        _ => return Err(invalid_shell(shell)),
    }
    let active = ActiveConfig::load(&paths.active_file())?;
    if active.shell_proxy.enabled {
        env(paths, shell)
    } else {
        noenv(shell)
    }
}

fn print_env(shell: &str, http_port: u16, socks_port: u16, bypass: &str) {
    let http = format!("http://127.0.0.1:{http_port}");
    let socks = format!("socks5h://127.0.0.1:{socks_port}");
    match shell {
        "fish" => {
            println!("set -gx http_proxy {};", fish_quote(&http));
            println!("set -gx https_proxy $http_proxy;");
            println!("set -gx all_proxy {};", fish_quote(&socks));
            println!("set -gx HTTP_PROXY $http_proxy;");
            println!("set -gx HTTPS_PROXY $https_proxy;");
            println!("set -gx ALL_PROXY $all_proxy;");
            println!("set -gx NO_PROXY {};", fish_quote(bypass));
            println!("set -gx no_proxy $NO_PROXY;");
        }
        "bash" | "zsh" => {
            println!("export http_proxy={}", shell_quote(&http));
            println!("export https_proxy=\"$http_proxy\"");
            println!("export all_proxy={}", shell_quote(&socks));
            println!("export HTTP_PROXY=\"$http_proxy\"");
            println!("export HTTPS_PROXY=\"$https_proxy\"");
            println!("export ALL_PROXY=\"$all_proxy\"");
            println!("export NO_PROXY={}", shell_quote(bypass));
            println!("export no_proxy=\"$NO_PROXY\"");
        }
        _ => unreachable!("shell validated by CLI"),
    }
}

fn proxy_ports(paths: &AppPaths, runtime: &RuntimeConfig) -> Result<(u16, u16), io::Error> {
    let active = ActiveConfig::load(&paths.active_file())?;
    if active.current.core.is_empty() {
        return Ok((runtime.proxy.mixed_port, runtime.proxy.mixed_port));
    }
    let manifest = load_manifest(&paths.cores_dir().join(&active.current.core))?;
    let capabilities = manifest.capabilities.config;
    Ok((
        if capabilities.http_proxy {
            runtime.proxy.http_port
        } else {
            runtime.proxy.mixed_port
        },
        if capabilities.socks_proxy {
            runtime.proxy.socks_port
        } else {
            runtime.proxy.mixed_port
        },
    ))
}

fn apply_system_proxy(
    http_port: &str,
    socks_port: &str,
    bypass: &[String],
) -> Result<(), io::Error> {
    for args in [
        ["set", "org.gnome.system.proxy.http", "host", "127.0.0.1"],
        ["set", "org.gnome.system.proxy.http", "port", http_port],
        ["set", "org.gnome.system.proxy.https", "host", "127.0.0.1"],
        ["set", "org.gnome.system.proxy.https", "port", http_port],
        ["set", "org.gnome.system.proxy.socks", "host", "127.0.0.1"],
        ["set", "org.gnome.system.proxy.socks", "port", socks_port],
    ] {
        gsettings(&args)?;
    }
    let value = gvariant_array(bypass);
    gsettings(&["set", "org.gnome.system.proxy", "ignore-hosts", &value])?;
    // Switch modes only after every dependent value is valid and applied.
    gsettings(&["set", "org.gnome.system.proxy", "mode", "manual"])?;
    Ok(())
}

fn capture_system_proxy() -> Result<SystemProxyBackup, io::Error> {
    Ok(SystemProxyBackup {
        mode: gsettings_get("org.gnome.system.proxy", "mode")?,
        http_host: gsettings_get("org.gnome.system.proxy.http", "host")?,
        http_port: gsettings_get("org.gnome.system.proxy.http", "port")?,
        https_host: gsettings_get("org.gnome.system.proxy.https", "host")?,
        https_port: gsettings_get("org.gnome.system.proxy.https", "port")?,
        socks_host: gsettings_get("org.gnome.system.proxy.socks", "host")?,
        socks_port: gsettings_get("org.gnome.system.proxy.socks", "port")?,
        ignore_hosts: gsettings_get("org.gnome.system.proxy", "ignore-hosts")?,
    })
}

fn restore_system_proxy(backup: &SystemProxyBackup) -> Result<(), io::Error> {
    for args in [
        [
            "set",
            "org.gnome.system.proxy.http",
            "host",
            backup.http_host.as_str(),
        ],
        [
            "set",
            "org.gnome.system.proxy.http",
            "port",
            backup.http_port.as_str(),
        ],
        [
            "set",
            "org.gnome.system.proxy.https",
            "host",
            backup.https_host.as_str(),
        ],
        [
            "set",
            "org.gnome.system.proxy.https",
            "port",
            backup.https_port.as_str(),
        ],
        [
            "set",
            "org.gnome.system.proxy.socks",
            "host",
            backup.socks_host.as_str(),
        ],
        [
            "set",
            "org.gnome.system.proxy.socks",
            "port",
            backup.socks_port.as_str(),
        ],
        [
            "set",
            "org.gnome.system.proxy",
            "ignore-hosts",
            backup.ignore_hosts.as_str(),
        ],
    ] {
        gsettings(&args)?;
    }
    gsettings(&["set", "org.gnome.system.proxy", "mode", &backup.mode])?;
    Ok(())
}

fn load_backup(path: &std::path::Path) -> Result<Option<SystemProxyBackup>, io::Error> {
    let content = match fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    serde_json::from_slice(&content)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn gsettings_get(schema: &str, key: &str) -> Result<String, io::Error> {
    let output = gsettings(&["get", schema, key])?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn bypass_items(
    paths: &AppPaths,
    enabled: bool,
    gsettings_mode: bool,
) -> Result<Vec<String>, io::Error> {
    if !enabled {
        return Ok(Vec::new());
    }
    let settings = Settings::load(&paths.settings_file())?;
    if !settings.bypass.enabled {
        return Ok(Vec::new());
    }
    let mut items = settings.bypass.inline;
    let content = fs::read_to_string(paths.bypass_file()).unwrap_or_default();
    items.extend(content.lines().filter_map(|line| {
        let value = line.split('#').next().unwrap_or_default().trim();
        (!value.is_empty()).then(|| value.to_owned())
    }));
    for item in &mut items {
        if gsettings_mode && item.starts_with('.') {
            *item = format!("*{item}");
        } else if !gsettings_mode && item.starts_with("*.") {
            *item = format!(".{}", item.trim_start_matches("*."));
        }
    }
    items.sort();
    items.dedup();
    Ok(items)
}

fn update_active(
    paths: &AppPaths,
    update: impl FnOnce(&mut ActiveConfig),
) -> Result<(), io::Error> {
    let _lock = AppLock::acquire(&paths.lock_file())?;
    let mut active = ActiveConfig::load(&paths.active_file())?;
    update(&mut active);
    active.save(&paths.active_file())
}

fn gsettings(args: &[&str]) -> Result<Output, io::Error> {
    let command = env::var_os("TZ_GSETTINGS_BIN").unwrap_or_else(|| "gsettings".into());
    let output = Command::new(command).args(args).output().map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "当前 v0.1 system proxy 需要 GNOME gsettings",
            )
        } else {
            error
        }
    })?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(io::Error::other(format!(
            "gsettings 执行失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn gvariant_array(items: &[String]) -> String {
    let items = items
        .iter()
        .map(|item| format!("'{}'", item.replace('\\', "\\\\").replace('\'', "\\'")))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn fish_quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn invalid_shell(shell: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("不支持的 shell `{shell}`"),
    )
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

#[cfg(test)]
mod tests {
    use super::{fish_quote, gvariant_array, shell_quote};

    #[test]
    fn escapes_shell_and_gvariant_values() {
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
        assert_eq!(fish_quote("a'b"), "'a\\'b'");
        assert_eq!(
            gvariant_array(&["localhost".into(), "*.local".into()]),
            "['localhost', '*.local']"
        );
    }
}
