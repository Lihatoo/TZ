use std::{
    fs, io,
    io::{IsTerminal, Write},
    net::IpAddr,
};

use crate::domain::{RuntimeConfig, Settings};
use crate::platform::{AppLock, AppPaths};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Settings,
    Runtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    Bool,
    Port,
    PositiveInteger,
    Ip,
    LogLevel,
    ProxyMode,
    TunStack,
}

#[derive(Debug, Clone, Copy)]
struct SettingSpec {
    key: &'static str,
    target: Target,
    kind: ValueKind,
}

const SPECS: &[SettingSpec] = &[
    spec("bypass.enabled", Target::Settings, ValueKind::Bool),
    spec("log.level", Target::Settings, ValueKind::LogLevel),
    spec(
        "log.max_size_mb",
        Target::Settings,
        ValueKind::PositiveInteger,
    ),
    spec("proxy.mode", Target::Runtime, ValueKind::ProxyMode),
    spec("proxy.listen", Target::Runtime, ValueKind::Ip),
    spec("proxy.mixed_port", Target::Runtime, ValueKind::Port),
    spec("proxy.http_port", Target::Runtime, ValueKind::Port),
    spec("proxy.socks_port", Target::Runtime, ValueKind::Port),
    spec("proxy.allow_lan", Target::Runtime, ValueKind::Bool),
    spec("proxy.ipv6", Target::Runtime, ValueKind::Bool),
    spec("api.enabled", Target::Runtime, ValueKind::Bool),
    spec("api.listen", Target::Runtime, ValueKind::Ip),
    spec("api.port", Target::Runtime, ValueKind::Port),
    spec("dns.enabled", Target::Runtime, ValueKind::Bool),
    spec("dns.listen", Target::Runtime, ValueKind::Ip),
    spec("dns.port", Target::Runtime, ValueKind::Port),
    spec("dns.ipv6", Target::Runtime, ValueKind::Bool),
    spec("tun.stack", Target::Runtime, ValueKind::TunStack),
    spec("tun.auto_route", Target::Runtime, ValueKind::Bool),
    spec(
        "tun.auto_detect_interface",
        Target::Runtime,
        ValueKind::Bool,
    ),
    spec("tun.dns_hijack", Target::Runtime, ValueKind::Bool),
];

const fn spec(key: &'static str, target: Target, kind: ValueKind) -> SettingSpec {
    SettingSpec { key, target, kind }
}

pub fn interactive(paths: &AppPaths) -> Result<(), io::Error> {
    if !io::stdin().is_terminal() {
        return list(paths);
    }
    let settings = Settings::load(&paths.settings_file())?;
    let runtime = RuntimeConfig::load(&paths.runtime_file())?;
    for (index, spec) in SPECS.iter().enumerate() {
        println!(
            "{:>2}) {:<28} {}",
            index + 1,
            spec.key,
            get_value(spec.key, &settings, &runtime)?
        );
    }
    let selected = prompt("选择项目（0 取消）: ")?;
    let number = selected
        .parse::<usize>()
        .map_err(|_| invalid("请输入列表中的数字"))?;
    if number == 0 {
        return Ok(());
    }
    let spec = SPECS
        .get(number.saturating_sub(1))
        .ok_or_else(|| invalid("选择超出范围"))?;
    let value = prompt(&format!("{} 新值: ", spec.key))?;
    set(paths, spec.key, Some(&value))
}

pub fn list(paths: &AppPaths) -> Result<(), io::Error> {
    let settings = Settings::load(&paths.settings_file())?;
    let runtime = RuntimeConfig::load(&paths.runtime_file())?;
    for spec in SPECS {
        println!(
            "{:<28} {:<8} {}",
            spec.key,
            kind_name(spec.kind),
            get_value(spec.key, &settings, &runtime)?
        );
    }
    Ok(())
}

pub fn get(paths: &AppPaths, key: &str) -> Result<(), io::Error> {
    find_spec(key)?;
    let settings = Settings::load(&paths.settings_file())?;
    let runtime = RuntimeConfig::load(&paths.runtime_file())?;
    println!("{}", get_value(key, &settings, &runtime)?);
    Ok(())
}

pub fn set(paths: &AppPaths, key: &str, value: Option<&str>) -> Result<(), io::Error> {
    let spec = find_spec(key)?;
    let value = match value {
        Some(value) => value.to_owned(),
        None if io::stdin().is_terminal() => prompt(&format!("{key} 新值: "))?,
        None => return Err(invalid("非交互调用必须提供 value")),
    };

    let _lock = AppLock::acquire(&paths.lock_file())?;
    let mut settings = Settings::load(&paths.settings_file())?;
    let mut runtime = RuntimeConfig::load(&paths.runtime_file())?;
    set_value(key, &value, &mut settings, &mut runtime)?;
    validate_runtime(&runtime)?;
    match spec.target {
        Target::Settings => settings.save(&paths.settings_file())?,
        Target::Runtime => runtime.save(&paths.runtime_file())?,
    }
    invalidate_generated(paths)?;
    println!("已保存 {key}={value}；需要重新 build/start 后生效。");
    Ok(())
}

pub fn reset(paths: &AppPaths, key: Option<&str>) -> Result<(), io::Error> {
    let _lock = AppLock::acquire(&paths.lock_file())?;
    let mut settings = Settings::load(&paths.settings_file())?;
    let mut runtime = RuntimeConfig::load(&paths.runtime_file())?;
    let defaults_settings = Settings::default();
    let defaults_runtime = RuntimeConfig::default();

    match key {
        Some(key) => {
            let spec = find_spec(key)?;
            let value = get_value(key, &defaults_settings, &defaults_runtime)?;
            set_value(key, &value, &mut settings, &mut runtime)?;
            validate_runtime(&runtime)?;
            match spec.target {
                Target::Settings => settings.save(&paths.settings_file())?,
                Target::Runtime => runtime.save(&paths.runtime_file())?,
            }
            println!("已恢复 {key}={value}；需要重新 build/start 后生效。");
        }
        None => {
            for spec in SPECS {
                let value = get_value(spec.key, &defaults_settings, &defaults_runtime)?;
                set_value(spec.key, &value, &mut settings, &mut runtime)?;
            }
            validate_runtime(&runtime)?;
            settings.save(&paths.settings_file())?;
            runtime.save(&paths.runtime_file())?;
            println!("已恢复全部公开设置；需要重新 build/start 后生效。");
        }
    }
    invalidate_generated(paths)?;
    Ok(())
}

fn find_spec(key: &str) -> Result<&'static SettingSpec, io::Error> {
    SPECS
        .iter()
        .find(|spec| spec.key == key)
        .ok_or_else(|| invalid(format!("未知 setting key `{key}`")))
}

fn get_value(key: &str, settings: &Settings, runtime: &RuntimeConfig) -> Result<String, io::Error> {
    let value = match key {
        "bypass.enabled" => settings.bypass.enabled.to_string(),
        "log.level" => settings.log.level.clone(),
        "log.max_size_mb" => settings.log.max_size_mb.to_string(),
        "proxy.mode" => runtime.proxy.mode.clone(),
        "proxy.listen" => runtime.proxy.listen.clone(),
        "proxy.mixed_port" => runtime.proxy.mixed_port.to_string(),
        "proxy.http_port" => runtime.proxy.http_port.to_string(),
        "proxy.socks_port" => runtime.proxy.socks_port.to_string(),
        "proxy.allow_lan" => runtime.proxy.allow_lan.to_string(),
        "proxy.ipv6" => runtime.proxy.ipv6.to_string(),
        "api.enabled" => runtime.api.enabled.to_string(),
        "api.listen" => runtime.api.listen.clone(),
        "api.port" => runtime.api.port.to_string(),
        "dns.enabled" => runtime.dns.enabled.to_string(),
        "dns.listen" => runtime.dns.listen.clone(),
        "dns.port" => runtime.dns.port.to_string(),
        "dns.ipv6" => runtime.dns.ipv6.to_string(),
        "tun.stack" => runtime.tun.stack.clone(),
        "tun.auto_route" => runtime.tun.auto_route.to_string(),
        "tun.auto_detect_interface" => runtime.tun.auto_detect_interface.to_string(),
        "tun.dns_hijack" => runtime.tun.dns_hijack.to_string(),
        _ => return Err(invalid(format!("未知 setting key `{key}`"))),
    };
    Ok(value)
}

fn set_value(
    key: &str,
    value: &str,
    settings: &mut Settings,
    runtime: &mut RuntimeConfig,
) -> Result<(), io::Error> {
    let spec = find_spec(key)?;
    validate_value(spec.kind, value)?;
    match key {
        "bypass.enabled" => settings.bypass.enabled = parse_bool(value)?,
        "log.level" => settings.log.level = value.to_ascii_lowercase(),
        "log.max_size_mb" => settings.log.max_size_mb = value.parse().map_err(parse_error)?,
        "proxy.mode" => runtime.proxy.mode = value.to_ascii_lowercase(),
        "proxy.listen" => runtime.proxy.listen = normalize_ip(value)?,
        "proxy.mixed_port" => runtime.proxy.mixed_port = parse_port(value)?,
        "proxy.http_port" => runtime.proxy.http_port = parse_port(value)?,
        "proxy.socks_port" => runtime.proxy.socks_port = parse_port(value)?,
        "proxy.allow_lan" => runtime.proxy.allow_lan = parse_bool(value)?,
        "proxy.ipv6" => runtime.proxy.ipv6 = parse_bool(value)?,
        "api.enabled" => runtime.api.enabled = parse_bool(value)?,
        "api.listen" => runtime.api.listen = normalize_ip(value)?,
        "api.port" => runtime.api.port = parse_port(value)?,
        "dns.enabled" => runtime.dns.enabled = parse_bool(value)?,
        "dns.listen" => runtime.dns.listen = normalize_ip(value)?,
        "dns.port" => runtime.dns.port = parse_port(value)?,
        "dns.ipv6" => runtime.dns.ipv6 = parse_bool(value)?,
        "tun.stack" => runtime.tun.stack = value.to_ascii_lowercase(),
        "tun.auto_route" => runtime.tun.auto_route = parse_bool(value)?,
        "tun.auto_detect_interface" => runtime.tun.auto_detect_interface = parse_bool(value)?,
        "tun.dns_hijack" => runtime.tun.dns_hijack = parse_bool(value)?,
        _ => return Err(invalid(format!("未知 setting key `{key}`"))),
    }
    Ok(())
}

fn validate_value(kind: ValueKind, value: &str) -> Result<(), io::Error> {
    match kind {
        ValueKind::Bool => {
            parse_bool(value)?;
        }
        ValueKind::Port => {
            parse_port(value)?;
        }
        ValueKind::PositiveInteger => {
            let number: u32 = value.parse().map_err(parse_error)?;
            if number == 0 || number > 4096 {
                return Err(invalid("数值必须在 1..=4096"));
            }
        }
        ValueKind::Ip => {
            normalize_ip(value)?;
        }
        ValueKind::LogLevel => {
            if !matches!(
                value.to_ascii_lowercase().as_str(),
                "error" | "warn" | "info" | "debug" | "trace"
            ) {
                return Err(invalid("日志级别必须是 error|warn|info|debug|trace"));
            }
        }
        ValueKind::ProxyMode => {
            if !matches!(
                value.to_ascii_lowercase().as_str(),
                "rule" | "global" | "direct"
            ) {
                return Err(invalid("代理模式必须是 rule|global|direct"));
            }
        }
        ValueKind::TunStack => {
            if !matches!(
                value.to_ascii_lowercase().as_str(),
                "system" | "gvisor" | "mixed"
            ) {
                return Err(invalid("TUN stack 必须是 system|gvisor|mixed"));
            }
        }
    }
    Ok(())
}

fn validate_runtime(runtime: &RuntimeConfig) -> Result<(), io::Error> {
    let ports = [
        ("proxy.mixed_port", runtime.proxy.mixed_port),
        ("proxy.http_port", runtime.proxy.http_port),
        ("proxy.socks_port", runtime.proxy.socks_port),
        ("api.port", runtime.api.port),
        ("dns.port", runtime.dns.port),
    ];
    for (index, (left_name, left)) in ports.iter().enumerate() {
        for (right_name, right) in ports.iter().skip(index + 1) {
            if left == right {
                return Err(invalid(format!(
                    "端口冲突：{left_name} 和 {right_name} 都是 {left}"
                )));
            }
        }
    }
    Ok(())
}

fn parse_bool(value: &str) -> Result<bool, io::Error> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "on" => Ok(true),
        "false" | "off" => Ok(false),
        _ => Err(invalid("布尔值必须是 on|off|true|false")),
    }
}

fn parse_port(value: &str) -> Result<u16, io::Error> {
    let port: u16 = value.parse().map_err(parse_error)?;
    if port == 0 {
        return Err(invalid("端口必须在 1..=65535"));
    }
    Ok(port)
}

fn normalize_ip(value: &str) -> Result<String, io::Error> {
    value
        .parse::<IpAddr>()
        .map(|ip| ip.to_string())
        .map_err(|_| invalid(format!("无效 IP 地址 `{value}`")))
}

fn parse_error(error: std::num::ParseIntError) -> io::Error {
    invalid(error.to_string())
}

fn invalidate_generated(paths: &AppPaths) -> Result<(), io::Error> {
    let generated = paths.generated_dir();
    if generated.is_dir() {
        fs::remove_dir_all(&generated)?;
    }
    fs::create_dir_all(generated)
}

fn prompt(message: &str) -> Result<String, io::Error> {
    print!("{message}");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_owned())
}

fn kind_name(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Bool => "bool",
        ValueKind::Port => "port",
        ValueKind::PositiveInteger => "integer",
        ValueKind::Ip => "ip",
        ValueKind::LogLevel | ValueKind::ProxyMode | ValueKind::TunStack => "enum",
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::{SPECS, set_value, validate_runtime};
    use crate::domain::{RuntimeConfig, Settings};
    use std::collections::HashSet;

    #[test]
    fn registry_keys_are_unique() {
        let mut keys = HashSet::new();
        assert!(SPECS.iter().all(|spec| keys.insert(spec.key)));
    }

    #[test]
    fn accepts_typed_values_and_rejects_unknown_keys() {
        let mut settings = Settings::default();
        let mut runtime = RuntimeConfig::default();
        set_value("proxy.mode", "global", &mut settings, &mut runtime).unwrap();
        set_value("proxy.allow_lan", "on", &mut settings, &mut runtime).unwrap();
        assert_eq!(runtime.proxy.mode, "global");
        assert!(runtime.proxy.allow_lan);
        assert!(set_value("active.tun", "on", &mut settings, &mut runtime).is_err());
    }

    #[test]
    fn rejects_port_conflicts() {
        let mut runtime = RuntimeConfig::default();
        runtime.api.port = runtime.proxy.mixed_port;
        assert!(validate_runtime(&runtime).is_err());
    }
}
