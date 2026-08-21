use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Map, Value, json};

use crate::{
    domain::{ActiveConfig, CoreDescriptor, ProfilesIndex, RuntimeConfig, Settings, load_manifest},
    platform::{AppPaths, atomic_write_private},
};

const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct BuiltConfig {
    pub core: CoreDescriptor,
    pub profile_name: String,
    pub config_path: PathBuf,
    pub workdir: PathBuf,
}

pub fn build(paths: &AppPaths) -> Result<BuiltConfig, io::Error> {
    let active = ActiveConfig::load(&paths.active_file())?;
    if active.current.core.is_empty() {
        return Err(invalid(
            "未选择 core；请先运行 `tz core list` 或 `tz core use <name>`",
        ));
    }
    let core_dir = paths.cores_dir().join(&active.current.core);
    let manifest = load_manifest(&core_dir)?;
    let core = CoreDescriptor {
        name: active.current.core.clone(),
        dir: core_dir,
        manifest,
    };
    let index = ProfilesIndex::load(&paths.profiles_file())?;
    let profile_name = index
        .current
        .get(&core.manifest.core.family)
        .ok_or_else(|| {
            invalid(format!(
                "未选择 {} profile；请先运行 `tz profile list`",
                core.manifest.core.family
            ))
        })?
        .clone();
    let profile = index
        .profiles
        .iter()
        .find(|profile| profile.name == profile_name && profile.family == core.manifest.core.family)
        .ok_or_else(|| {
            invalid(format!(
                "上次使用的 {} profile `{profile_name}` 不可用；请运行 `tz profile list` 重新选择",
                core.manifest.core.family
            ))
        })?;
    let source_path = paths.profiles_dir().join(&profile.source_file);
    let source = fs::read(&source_path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "上次使用的 profile `{profile_name}` 缺少 source 文件 {}: {error}",
                source_path.display()
            ),
        )
    })?;
    let runtime = RuntimeConfig::load(&paths.runtime_file())?;
    let settings = Settings::load(&paths.settings_file())?;
    let bypass = load_bypass(paths, &settings)?;
    let generated = match core.manifest.core.family.as_str() {
        "clash" => build_clash(&source, &runtime, &settings, &active, &bypass)?,
        "sing-box" => build_sing_box(&source, &runtime, &settings, &active, &bypass)?,
        family => return Err(invalid(format!("不支持的 core family `{family}`"))),
    };

    let generated_dir = paths.generated_dir().join(&core.name);
    let workdir = paths.core_workdir(&core.name);
    fs::create_dir_all(&generated_dir)?;
    fs::create_dir_all(&workdir)?;
    if core.manifest.core.family == "clash" && requires_clash_geodata(&generated)? {
        install_clash_geodata(&core, &workdir)?;
    }
    let config_path = generated_dir.join(&core.manifest.runtime.entrypoint);
    atomic_write_private(&config_path, &generated)?;
    Ok(BuiltConfig {
        core,
        profile_name,
        config_path,
        workdir,
    })
}

pub fn check(paths: &AppPaths) -> Result<BuiltConfig, io::Error> {
    let built = build(paths)?;
    let command = built
        .core
        .manifest
        .commands
        .check
        .as_ref()
        .ok_or_else(|| invalid(format!("core `{}` 未声明 check 命令", built.core.name)))?;
    let args = built
        .core
        .render_args(&command.args, &built.config_path, &built.workdir);
    let mut child = Command::new(built.core.binary_path())
        .args(args)
        .current_dir(&built.workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            let output = child.wait_with_output()?;
            if !status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                let detail = if stderr.is_empty() { stdout } else { stderr };
                return Err(invalid(format!("core 配置校验失败: {detail}")));
            }
            return Ok(built);
        }
        if started.elapsed() >= CHECK_TIMEOUT {
            let _ = child.kill();
            let output = child.wait_with_output()?;
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            let detail = if stderr.is_empty() { stdout } else { stderr };
            let message = if detail.is_empty() {
                "core 配置校验超时（10s）".into()
            } else {
                format!("core 配置校验超时（10s）: {detail}")
            };
            return Err(io::Error::new(io::ErrorKind::TimedOut, message));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn requires_clash_geodata(generated: &[u8]) -> Result<bool, io::Error> {
    let root: Value = serde_yaml::from_slice(generated).map_err(parse_error)?;
    Ok(contains_geo_reference(&root))
}

fn contains_geo_reference(value: &Value) -> bool {
    match value {
        Value::String(value) => {
            let value = value.to_ascii_uppercase();
            value.contains("GEOIP") || value.contains("GEOSITE")
        }
        Value::Array(values) => values.iter().any(contains_geo_reference),
        Value::Object(values) => values.iter().any(|(key, value)| {
            let key = key.to_ascii_uppercase();
            key.contains("GEOIP") || key.contains("GEOSITE") || contains_geo_reference(value)
        }),
        _ => false,
    }
}

fn install_clash_geodata(core: &CoreDescriptor, workdir: &Path) -> Result<(), io::Error> {
    for name in ["Country.mmdb", "GeoSite.dat"] {
        let target = workdir.join(name);
        if target.is_file() {
            continue;
        }
        let source = core.dir.join(name);
        if !source.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "当前配置需要 Mihomo Geo 数据，但 core 包缺少 `{name}`；请使用带 Geo 数据的 Mihomo core 包，或先打开其他代理后补齐该文件"
                ),
            ));
        }
        copy_runtime_asset(&source, &target)?;
    }
    Ok(())
}

fn copy_runtime_asset(source: &Path, target: &Path) -> Result<(), io::Error> {
    let temporary = target.with_file_name(format!(
        ".{}.{}.tmp",
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("asset"),
        std::process::id()
    ));
    fs::copy(source, &temporary)?;
    if let Err(error) = fs::rename(&temporary, target) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

fn build_clash(
    source: &[u8],
    runtime: &RuntimeConfig,
    settings: &Settings,
    active: &ActiveConfig,
    bypass: &[String],
) -> Result<Vec<u8>, io::Error> {
    let mut root: Value = serde_yaml::from_slice(source).map_err(parse_error)?;
    let object = object_mut(&mut root)?;
    object.insert("mixed-port".into(), json!(runtime.proxy.mixed_port));
    object.insert("port".into(), json!(runtime.proxy.http_port));
    object.insert("socks-port".into(), json!(runtime.proxy.socks_port));
    object.insert("allow-lan".into(), json!(runtime.proxy.allow_lan));
    object.insert("bind-address".into(), json!(runtime.proxy.listen));
    object.insert("mode".into(), json!(runtime.proxy.mode));
    object.insert("ipv6".into(), json!(runtime.proxy.ipv6));
    object.insert(
        "log-level".into(),
        json!(match settings.log.level.as_str() {
            // TZ uses the cross-core spelling accepted by sing-box.
            "warn" => "warning",
            level => level,
        }),
    );
    if runtime.api.enabled {
        object.insert(
            "external-controller".into(),
            json!(format!("{}:{}", runtime.api.listen, runtime.api.port)),
        );
    } else {
        object.remove("external-controller");
    }
    object.insert(
        "tun".into(),
        json!({
            "enable": active.tun.enabled,
            "stack": runtime.tun.stack,
            "auto-route": runtime.tun.auto_route,
            "auto-detect-interface": runtime.tun.auto_detect_interface,
            "dns-hijack": if runtime.tun.dns_hijack { json!(["any:53"]) } else { json!([]) },
        }),
    );
    if runtime.dns.enabled {
        let dns = object.entry("dns").or_insert_with(|| json!({}));
        let dns = object_mut(dns)?;
        dns.insert("enable".into(), json!(true));
        dns.insert(
            "listen".into(),
            json!(format!("{}:{}", runtime.dns.listen, runtime.dns.port)),
        );
        dns.insert("ipv6".into(), json!(runtime.dns.ipv6));
        dns.entry("nameserver")
            .or_insert_with(|| json!(["223.5.5.5", "119.29.29.29"]));
    } else {
        object.remove("dns");
    }

    let proxy_names: Vec<_> = object
        .get("proxies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|proxy| proxy.get("name").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    if object
        .get("proxy-groups")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        let mut choices = proxy_names;
        choices.push("DIRECT".into());
        object.insert(
            "proxy-groups".into(),
            json!([{"name":"Proxy", "type":"select", "proxies":choices}]),
        );
    }
    let mut rules: Vec<Value> = bypass
        .iter()
        .map(|item| Value::String(clash_bypass_rule(item)))
        .collect();
    if let Some(existing) = object.get("rules").and_then(Value::as_array) {
        rules.extend(existing.iter().cloned());
    }
    if rules
        .iter()
        .all(|rule| !rule.as_str().is_some_and(|rule| rule.starts_with("MATCH,")))
    {
        rules.push(json!("MATCH,Proxy"));
    }
    object.insert("rules".into(), Value::Array(rules));
    serde_yaml::to_string(&root)
        .map(String::into_bytes)
        .map_err(parse_error)
}

fn build_sing_box(
    source: &[u8],
    runtime: &RuntimeConfig,
    settings: &Settings,
    active: &ActiveConfig,
    bypass: &[String],
) -> Result<Vec<u8>, io::Error> {
    let mut root: Value = serde_json::from_slice(source).map_err(parse_error)?;
    let object = object_mut(&mut root)?;
    object.insert("log".into(), json!({"level": settings.log.level}));

    let mut outbounds = object
        .remove("outbounds")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let mut tags: Vec<String> = outbounds
        .iter()
        .filter_map(|item| item.get("tag").and_then(Value::as_str).map(str::to_owned))
        .collect();
    if !tags.iter().any(|tag| tag == "DIRECT") {
        outbounds.push(json!({"type":"direct", "tag":"DIRECT"}));
        tags.push("DIRECT".into());
    }
    if !tags.iter().any(|tag| tag == "Proxy") {
        let choices: Vec<_> = outbounds
            .iter()
            .filter(|item| {
                !matches!(
                    item.get("type").and_then(Value::as_str),
                    Some("direct" | "block" | "dns" | "selector")
                )
            })
            .filter_map(|item| item.get("tag").and_then(Value::as_str))
            .collect();
        outbounds.push(json!({"type":"selector", "tag":"Proxy", "outbounds":choices}));
    }
    object.insert("outbounds".into(), Value::Array(outbounds));

    let mut inbounds = vec![json!({
        "type":"mixed", "tag":"mixed-in", "listen":runtime.proxy.listen,
        "listen_port":runtime.proxy.mixed_port,
    })];
    if active.tun.enabled {
        inbounds.push(json!({
            "type":"tun", "tag":"tun-in", "address":["172.19.0.1/30", "fdfe:dcba:9876::1/126"],
            "auto_route":runtime.tun.auto_route,
        }));
    }
    object.insert("inbounds".into(), Value::Array(inbounds));

    let route = object.entry("route").or_insert_with(|| json!({}));
    let route = object_mut(route)?;
    route.insert(
        "auto_detect_interface".into(),
        json!(runtime.tun.auto_detect_interface),
    );
    route.insert(
        "final".into(),
        json!(match runtime.proxy.mode.as_str() {
            "direct" => "DIRECT",
            _ => "Proxy",
        }),
    );
    let mut rules = route
        .remove("rules")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let mut system_rules: Vec<_> = bypass
        .iter()
        .map(|item| sing_box_bypass_rule(item))
        .collect();
    system_rules.append(&mut rules);
    route.insert("rules".into(), Value::Array(system_rules));

    if runtime.api.enabled {
        let experimental = object.entry("experimental").or_insert_with(|| json!({}));
        let experimental = object_mut(experimental)?;
        experimental.insert(
            "clash_api".into(),
            json!({
                "external_controller": format!("{}:{}", runtime.api.listen, runtime.api.port)
            }),
        );
    }
    serde_json::to_vec_pretty(&root).map_err(parse_error)
}

fn load_bypass(paths: &AppPaths, settings: &Settings) -> Result<Vec<String>, io::Error> {
    if !settings.bypass.enabled {
        return Ok(Vec::new());
    }
    let mut items = settings.bypass.inline.clone();
    let content = fs::read_to_string(paths.bypass_file()).unwrap_or_default();
    items.extend(content.lines().filter_map(|line| {
        let value = line.split('#').next().unwrap_or_default().trim();
        (!value.is_empty()).then(|| value.to_owned())
    }));
    items.sort();
    items.dedup();
    Ok(items)
}

fn clash_bypass_rule(item: &str) -> String {
    if item.contains('/') {
        format!("IP-CIDR,{item},DIRECT,no-resolve")
    } else if item.starts_with("*.") || item.starts_with('.') {
        format!(
            "DOMAIN-SUFFIX,{},DIRECT",
            item.trim_start_matches("*.").trim_start_matches('.')
        )
    } else {
        format!("DOMAIN,{item},DIRECT")
    }
}

fn sing_box_bypass_rule(item: &str) -> Value {
    if item.contains('/') {
        json!({"ip_cidr":[item], "action":"route", "outbound":"DIRECT"})
    } else if item.starts_with("*.") || item.starts_with('.') {
        json!({"domain_suffix":[item.trim_start_matches("*.").trim_start_matches('.')], "action":"route", "outbound":"DIRECT"})
    } else {
        json!({"domain":[item], "action":"route", "outbound":"DIRECT"})
    }
}

fn object_mut(value: &mut Value) -> Result<&mut Map<String, Value>, io::Error> {
    value
        .as_object_mut()
        .ok_or_else(|| invalid("配置根节点必须是 object"))
}

fn parse_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{build_clash, build_sing_box, requires_clash_geodata};
    use crate::domain::{ActiveConfig, RuntimeConfig, Settings};

    #[test]
    fn clash_maps_cross_core_warning_level() {
        let source = br#"
proxies:
  - name: test
    type: socks5
    server: 127.0.0.1
    port: 1080
"#;
        let generated = build_clash(
            source,
            &RuntimeConfig::default(),
            &Settings::default(),
            &ActiveConfig::default(),
            &[],
        )
        .unwrap();
        let root: Value = serde_yaml::from_slice(&generated).unwrap();
        assert_eq!(root["log-level"], "warning");
        assert_eq!(root["proxy-groups"][0]["name"], "Proxy");
        assert_eq!(
            root["rules"].as_array().unwrap().last().unwrap(),
            "MATCH,Proxy"
        );
    }

    #[test]
    fn detects_clash_geo_resource_references() {
        assert!(requires_clash_geodata(b"rules:\n  - GEOIP,CN,DIRECT\n").unwrap());
        assert!(
            requires_clash_geodata(b"dns:\n  nameserver-policy:\n    geosite:cn: 223.5.5.5\n")
                .unwrap()
        );
        assert!(!requires_clash_geodata(b"rules:\n  - MATCH,Proxy\n").unwrap());
    }

    #[test]
    fn sing_box_builds_clash_api_and_selector() {
        let source = br#"{
          "outbounds": [
            {"type":"socks", "tag":"test", "server":"127.0.0.1", "server_port":1080}
          ]
        }"#;
        let mut active = ActiveConfig::default();
        active.tun.enabled = true;
        let generated = build_sing_box(
            source,
            &RuntimeConfig::default(),
            &Settings::default(),
            &active,
            &[],
        )
        .unwrap();
        let root: Value = serde_json::from_slice(&generated).unwrap();
        assert_eq!(root["log"]["level"], "warn");
        assert_eq!(root["inbounds"][0]["type"], "mixed");
        assert_eq!(root["inbounds"][1]["type"], "tun");
        assert!(root["inbounds"][1].get("auto_detect_interface").is_none());
        assert_eq!(root["route"]["auto_detect_interface"], true);
        assert_eq!(
            root["experimental"]["clash_api"]["external_controller"],
            "127.0.0.1:9189"
        );
        assert!(
            root["outbounds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| { item["type"] == "selector" && item["tag"] == "Proxy" })
        );
    }
}
