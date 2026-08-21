use std::{
    cmp::Ordering,
    fs::{self, OpenOptions},
    io::{self, IsTerminal, Write},
    net::{IpAddr, SocketAddr, TcpStream},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use reqwest::{Url, blocking::Client};
use serde_json::{Value, json};

use crate::{
    application::config,
    domain::{ActiveConfig, ConfigCapabilities, ProfilesIndex, RuntimeConfig, load_manifest},
    platform::{
        AppLock, AppPaths, ManagedProcess, atomic_write_private, ensure_not_running,
        ensure_owned_process, managed_process, terminate_process,
    },
};

const START_TIMEOUT: Duration = Duration::from_secs(5);
const READY_STABILITY: Duration = Duration::from_millis(300);
const PORT_CONNECT_TIMEOUT: Duration = Duration::from_millis(100);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_TEST_URL: &str = "https://www.gstatic.com/generate_204";
const DEFAULT_TEST_TIMEOUT_MS: u64 = 1800;

#[derive(Debug)]
struct NodeGroup {
    name: String,
    current: String,
    nodes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct NodeTestOptions<'a> {
    pub keyword: Option<&'a str>,
    pub url: &'a str,
    pub timeout_ms: u64,
    pub select: bool,
}

pub fn status(paths: &AppPaths) -> Result<(), io::Error> {
    let active = ActiveConfig::load(&paths.active_file())?;
    let core = if active.current.core.is_empty() {
        None
    } else {
        let manifest = load_manifest(&paths.cores_dir().join(&active.current.core))?;
        Some((active.current.core.as_str(), manifest))
    };
    if let Some((name, manifest)) = &core {
        println!(
            "core     {} version={} family={}",
            name, manifest.core.version, manifest.core.family
        );
        let profiles = ProfilesIndex::load(&paths.profiles_file())?;
        let profile = profiles
            .current
            .get(&manifest.core.family)
            .map(String::as_str)
            .unwrap_or("-");
        println!("profile  {profile} family={}", manifest.core.family);
    } else {
        println!("core     -");
        println!("profile  -");
    }
    println!(
        "features tun={} terminal={} system={}",
        on_off(active.tun.enabled),
        on_off(active.shell_proxy.enabled),
        on_off(active.system_proxy.enabled)
    );

    match managed_process(&paths.core_pid_file())? {
        ManagedProcess::Running(pid) => {
            println!("service  running pid={pid}");
            let runtime = RuntimeConfig::load(&paths.runtime_file())?;
            let node = fetch_group(&runtime)
                .map(|group| group.current)
                .unwrap_or_default();
            if node.is_empty() {
                println!("node     -");
            } else {
                match test_node_delay(&runtime, &node, DEFAULT_TEST_URL, DEFAULT_TEST_TIMEOUT_MS) {
                    Ok(delay) => println!("node     {node} delay={delay}ms"),
                    Err(_) => match cached_delay(paths, &node) {
                        Some(delay) => println!("node     {node} delay={delay}ms cached"),
                        None => println!("node     {node} delay=timeout"),
                    },
                }
            }
        }
        ManagedProcess::Stale(pid) => println!("service  stopped stale-pid={pid}"),
        ManagedProcess::NotRunning => println!("service  stopped"),
    }
    Ok(())
}

pub fn start(paths: &AppPaths) -> Result<(), io::Error> {
    let _lock = AppLock::acquire(&paths.lock_file())?;
    ensure_not_running(&paths.core_pid_file())?;
    remove_stale_pid(paths)?;

    let built = config::check(paths)?;
    fs::create_dir_all(paths.logs_dir())?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.core_log_file())?;
    let stderr = log.try_clone()?;
    let args = built.core.render_args(
        &built.core.manifest.commands.start.args,
        &built.config_path,
        &built.workdir,
    );
    let mut child = Command::new(built.core.binary_path())
        .args(args)
        .current_dir(&built.workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .spawn()?;
    let pid = i32::try_from(child.id()).map_err(|_| io::Error::other("core PID 超出范围"))?;
    atomic_write_private(&paths.core_pid_file(), format!("{pid}\n").as_bytes())?;

    let runtime = RuntimeConfig::load(&paths.runtime_file())?;
    let started = Instant::now();
    let mut ready_since = None;
    loop {
        if let Some(status) = child.try_wait()? {
            let _ = fs::remove_file(paths.core_pid_file());
            return Err(io::Error::other(format!(
                "core 启动后立即退出（{status}），请查看 {}",
                paths.core_log_file().display()
            )));
        }
        let api_ready = !runtime.api.enabled || fetch_group(&runtime).is_ok();
        let ports_ready = proxy_ports_ready(&runtime, &built.core.manifest.capabilities.config);
        if api_ready && ports_ready {
            let stable_since = ready_since.get_or_insert_with(Instant::now);
            if stable_since.elapsed() < READY_STABILITY {
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            if runtime.api.enabled
                && let Err(error) = restore_selected_nodes(paths, &runtime, &built.profile_name)
            {
                eprintln!("恢复节点选择失败: {error}");
            }
            println!(
                "已启动 {} profile={} pid={pid}",
                built.core.name, built.profile_name
            );
            return Ok(());
        } else {
            ready_since = None;
        }
        if started.elapsed() >= START_TIMEOUT {
            let _ = terminate_process(pid, false);
            let _ = child.wait();
            let _ = fs::remove_file(paths.core_pid_file());
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "core API 或代理端口启动超时，请查看 {}",
                    paths.core_log_file().display()
                ),
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn proxy_ports_ready(runtime: &RuntimeConfig, capabilities: &ConfigCapabilities) -> bool {
    let host = match runtime.proxy.listen.as_str() {
        "0.0.0.0" => IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        "::" => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        value => match value.parse() {
            Ok(address) => address,
            Err(_) => return false,
        },
    };
    required_proxy_ports(runtime, capabilities)
        .into_iter()
        .all(|port| {
            TcpStream::connect_timeout(&SocketAddr::new(host, port), PORT_CONNECT_TIMEOUT).is_ok()
        })
}

fn required_proxy_ports(runtime: &RuntimeConfig, capabilities: &ConfigCapabilities) -> Vec<u16> {
    let mut ports = Vec::with_capacity(3);
    if capabilities.mixed_proxy {
        ports.push(runtime.proxy.mixed_port);
    }
    if capabilities.http_proxy {
        ports.push(runtime.proxy.http_port);
    }
    if capabilities.socks_proxy {
        ports.push(runtime.proxy.socks_port);
    }
    ports
}

pub fn stop(paths: &AppPaths) -> Result<(), io::Error> {
    let _lock = AppLock::acquire(&paths.lock_file())?;
    let active = ActiveConfig::load(&paths.active_file())?;
    let state = managed_process(&paths.core_pid_file())?;
    let pid = match state {
        ManagedProcess::NotRunning => {
            println!("服务未运行");
            return Ok(());
        }
        ManagedProcess::Stale(_) => {
            fs::remove_file(paths.core_pid_file())?;
            println!("服务未运行，已清理陈旧 PID");
            return Ok(());
        }
        ManagedProcess::Running(pid) => pid,
    };
    if active.current.core.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "存在运行 PID，但 active.toml 没有当前 core；拒绝停止",
        ));
    }
    let manifest = load_manifest(&paths.cores_dir().join(&active.current.core))?;
    ensure_owned_process(
        pid,
        &paths
            .cores_dir()
            .join(&active.current.core)
            .join(&manifest.core.binary),
    )?;
    terminate_process(pid, false)?;
    let started = Instant::now();
    while crate::platform::process::is_process_alive(pid) {
        if started.elapsed() >= STOP_TIMEOUT {
            ensure_owned_process(
                pid,
                &paths
                    .cores_dir()
                    .join(&active.current.core)
                    .join(&manifest.core.binary),
            )?;
            terminate_process(pid, true)?;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    fs::remove_file(paths.core_pid_file())?;
    println!("已停止 {}", active.current.core);
    Ok(())
}

pub fn restart(paths: &AppPaths) -> Result<(), io::Error> {
    stop(paths)?;
    start(paths)
}

pub fn list(paths: &AppPaths, keyword: Option<&str>) -> Result<(), io::Error> {
    if !matches!(
        managed_process(&paths.core_pid_file())?,
        ManagedProcess::Running(_)
    ) {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "服务未运行，请先执行 `tz start`",
        ));
    }
    let runtime = RuntimeConfig::load(&paths.runtime_file())?;
    let group = fetch_group(&runtime)?;
    let needle = keyword.unwrap_or_default().to_lowercase();
    let nodes: Vec<_> = group
        .nodes
        .iter()
        .filter(|name| needle.is_empty() || name.to_lowercase().contains(&needle))
        .cloned()
        .collect();
    if nodes.is_empty() {
        println!("没有匹配的节点");
        return Ok(());
    }
    let results = measure_node_delays(&runtime, &nodes, DEFAULT_TEST_URL, DEFAULT_TEST_TIMEOUT_MS);
    save_speedtest(paths, DEFAULT_TEST_URL, DEFAULT_TEST_TIMEOUT_MS, &results)?;
    for (index, (node, delay)) in results.iter().enumerate() {
        let marker = if *node == group.current { "*" } else { " " };
        let delay = delay.map_or_else(|| "timeout".into(), |delay| format!("{delay}ms"));
        if io::stdin().is_terminal() {
            println!("{marker} {}) {node} {delay}", index + 1);
        } else {
            println!("{marker} {node} {delay}");
        }
    }
    if io::stdin().is_terminal() {
        print!("选择节点（0 保持当前）: ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let selected = input
            .trim()
            .parse::<usize>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "请输入列表中的数字"))?;
        if selected > results.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "选择超出范围"));
        }
        if selected > 0 {
            let node = &results[selected - 1].0;
            select_node(&runtime, &group.name, node)?;
            persist_selected_node(paths, &group.name, node)?;
            println!("当前节点: {node}");
        }
    }
    Ok(())
}

pub fn test_nodes(paths: &AppPaths, options: NodeTestOptions<'_>) -> Result<(), io::Error> {
    if !matches!(
        managed_process(&paths.core_pid_file())?,
        ManagedProcess::Running(_)
    ) {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "服务未运行，请先执行 `tz start`",
        ));
    }
    let test_url = Url::parse(options.url)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    if !matches!(test_url.scheme(), "http" | "https")
        || !test_url.username().is_empty()
        || test_url.password().is_some()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "测速 URL 必须是无凭据的 HTTP(S) URL",
        ));
    }
    if !(100..=60_000).contains(&options.timeout_ms) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "timeout 必须在 100..=60000 ms",
        ));
    }
    let runtime = RuntimeConfig::load(&paths.runtime_file())?;
    let group = fetch_group(&runtime)?;
    let needle = options.keyword.unwrap_or_default().to_lowercase();
    let nodes: Vec<_> = group
        .nodes
        .iter()
        .filter(|node| needle.is_empty() || node.to_lowercase().contains(&needle))
        .cloned()
        .collect();
    if nodes.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "没有匹配的节点"));
    }

    let results = measure_node_delays(&runtime, &nodes, options.url, options.timeout_ms);
    for (node, delay) in &results {
        let marker = if *node == group.current { "*" } else { " " };
        match delay {
            Some(delay) => println!("{marker} {node} {delay}ms"),
            None => println!("{marker} {node} timeout"),
        }
    }
    save_speedtest(paths, options.url, options.timeout_ms, &results)?;
    if options.select {
        let fastest = results
            .iter()
            .find_map(|(node, delay)| delay.map(|delay| (node, delay)))
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "全部节点测速超时"))?;
        select_node(&runtime, &group.name, fastest.0)?;
        persist_selected_node(paths, &group.name, fastest.0)?;
        println!("当前节点: {} {}ms", fastest.0, fastest.1);
    }
    Ok(())
}

fn measure_node_delays(
    runtime: &RuntimeConfig,
    nodes: &[String],
    url: &str,
    timeout_ms: u64,
) -> Vec<(String, Option<u64>)> {
    let mut results = Vec::with_capacity(nodes.len());
    for chunk in nodes.chunks(8) {
        thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|node| {
                    let node = node.clone();
                    scope.spawn(move || {
                        let delay = test_node_delay(runtime, &node, url, timeout_ms).ok();
                        (node, delay)
                    })
                })
                .collect();
            for (node, handle) in chunk.iter().zip(handles) {
                results.push(handle.join().unwrap_or_else(|_| (node.clone(), None)));
            }
        });
    }
    sort_node_delays(&mut results);
    results
}

fn sort_node_delays(results: &mut [(String, Option<u64>)]) {
    results.sort_by(|left, right| match (left.1, right.1) {
        (Some(a), Some(b)) => a.cmp(&b).then_with(|| left.0.cmp(&right.0)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => left.0.cmp(&right.0),
    });
}

fn fetch_group(runtime: &RuntimeConfig) -> Result<NodeGroup, io::Error> {
    if !runtime.api.enabled {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "core API 未启用",
        ));
    }
    let response = api_client()?
        .get(api_url(runtime, "proxies")?)
        .send()
        .map_err(http_error)?
        .error_for_status()
        .map_err(http_error)?;
    let root: Value = serde_json::from_str(&response.text().map_err(http_error)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let proxies = root
        .get("proxies")
        .and_then(Value::as_object)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "core API 缺少 proxies"))?;
    let selected = proxies
        .get("Proxy")
        .filter(|value| value.get("all").and_then(Value::as_array).is_some())
        .map(|value| ("Proxy", value))
        .or_else(|| {
            proxies.iter().find_map(|(name, value)| {
                value
                    .get("all")
                    .and_then(Value::as_array)
                    .filter(|items| !items.is_empty())
                    .map(|_| (name.as_str(), value))
            })
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "没有可选择的节点组"))?;
    let nodes = selected
        .1
        .get("all")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|name| is_concrete_proxy(proxies.get(*name)))
        .map(str::to_owned)
        .collect();
    Ok(NodeGroup {
        name: selected.0.to_owned(),
        current: selected
            .1
            .get("now")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        nodes,
    })
}

fn is_concrete_proxy(value: Option<&Value>) -> bool {
    let kind = value
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .replace(['-', '_'], "");
    !matches!(
        kind.as_str(),
        "direct"
            | "reject"
            | "selector"
            | "urltest"
            | "fallback"
            | "loadbalance"
            | "compatible"
            | "pass"
            | "block"
            | "dns"
    )
}

fn test_node_delay(
    runtime: &RuntimeConfig,
    node: &str,
    test_url: &str,
    timeout_ms: u64,
) -> Result<u64, io::Error> {
    let mut url = api_url(runtime, "proxies")?;
    url.path_segments_mut()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "无效 API 地址"))?
        .push(node)
        .push("delay");
    url.query_pairs_mut()
        .append_pair("timeout", &timeout_ms.to_string())
        .append_pair("url", test_url);
    let response = Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(timeout_ms + 500))
        .build()
        .map_err(http_error)?
        .get(url)
        .send()
        .map_err(http_error)?
        .error_for_status()
        .map_err(http_error)?;
    let root: Value = serde_json::from_str(&response.text().map_err(http_error)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    root.get("delay")
        .and_then(Value::as_u64)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "测速 API 缺少 delay"))
}

fn save_speedtest(
    paths: &AppPaths,
    url: &str,
    timeout_ms: u64,
    results: &[(String, Option<u64>)],
) -> Result<(), io::Error> {
    fs::create_dir_all(paths.speedtest_dir())?;
    let entries: Vec<_> = results
        .iter()
        .map(|(node, delay)| json!({"node": node, "delay_ms": delay}))
        .collect();
    let content = serde_json::to_vec_pretty(&json!({
        "tested_at": jiff::Timestamp::now().to_string(),
        "url": url,
        "timeout_ms": timeout_ms,
        "results": entries,
    }))
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    atomic_write_private(&paths.speedtest_dir().join("latest.json"), &content)
}

fn cached_delay(paths: &AppPaths, node: &str) -> Option<u64> {
    let content = fs::read(paths.speedtest_dir().join("latest.json")).ok()?;
    let root: Value = serde_json::from_slice(&content).ok()?;
    root.get("results")?
        .as_array()?
        .iter()
        .find(|entry| entry.get("node").and_then(Value::as_str) == Some(node))?
        .get("delay_ms")?
        .as_u64()
}

fn select_node(runtime: &RuntimeConfig, group: &str, node: &str) -> Result<(), io::Error> {
    let mut url = api_url(runtime, "proxies")?;
    url.path_segments_mut()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "无效 API 地址"))?
        .push(group);
    api_client()?
        .put(url)
        .header("content-type", "application/json")
        .body(json!({"name": node}).to_string())
        .send()
        .map_err(http_error)?
        .error_for_status()
        .map_err(http_error)?;
    Ok(())
}

fn persist_selected_node(paths: &AppPaths, group: &str, node: &str) -> Result<(), io::Error> {
    let _lock = AppLock::acquire(&paths.lock_file())?;
    let active = ActiveConfig::load(&paths.active_file())?;
    let manifest = load_manifest(&paths.cores_dir().join(&active.current.core))?;
    let mut index = ProfilesIndex::load(&paths.profiles_file())?;
    let profile_name = index
        .current
        .get(&manifest.core.family)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "当前 family 没有 profile"))?
        .clone();
    let profile = index
        .profiles
        .iter_mut()
        .find(|profile| profile.name == profile_name && profile.family == manifest.core.family)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "当前 profile 不存在"))?;
    profile
        .state
        .selected
        .insert(group.to_owned(), node.to_owned());
    index.save(&paths.profiles_file())
}

fn restore_selected_nodes(
    paths: &AppPaths,
    runtime: &RuntimeConfig,
    profile_name: &str,
) -> Result<(), io::Error> {
    let index = ProfilesIndex::load(&paths.profiles_file())?;
    let profile = index
        .find(profile_name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "当前 profile 不存在"))?;
    for (group, node) in &profile.state.selected {
        select_node(runtime, group, node)?;
    }
    Ok(())
}

fn api_client() -> Result<Client, io::Error> {
    Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(500))
        .build()
        .map_err(http_error)
}

fn api_url(runtime: &RuntimeConfig, resource: &str) -> Result<Url, io::Error> {
    let host = match runtime.api.listen.as_str() {
        "0.0.0.0" | "::" => "127.0.0.1",
        host => host,
    };
    Url::parse(&format!("http://{host}:{}/{resource}", runtime.api.port))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

fn remove_stale_pid(paths: &AppPaths) -> Result<(), io::Error> {
    if matches!(
        managed_process(&paths.core_pid_file())?,
        ManagedProcess::Stale(_)
    ) {
        fs::remove_file(paths.core_pid_file())?;
    }
    Ok(())
}

fn http_error(error: reqwest::Error) -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionRefused, error)
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use serde_json::json;

    use super::{is_concrete_proxy, proxy_ports_ready, sort_node_delays, test_node_delay};
    use crate::domain::{ConfigCapabilities, RuntimeConfig};

    fn proxy_capabilities() -> ConfigCapabilities {
        ConfigCapabilities {
            mixed_proxy: true,
            http_proxy: false,
            socks_proxy: false,
            api: true,
            dns: true,
            tun: true,
        }
    }

    #[test]
    fn classifies_selectors_and_real_proxy_nodes() {
        assert!(!is_concrete_proxy(Some(&json!({"type":"Direct"}))));
        assert!(!is_concrete_proxy(Some(&json!({"type":"URLTest"}))));
        assert!(is_concrete_proxy(Some(&json!({"type":"Shadowsocks"}))));
        assert!(is_concrete_proxy(Some(&json!({"type":"VLESS"}))));
    }

    #[test]
    fn sorts_successful_delays_before_timeouts() {
        let mut results = vec![
            ("timeout-b".into(), None),
            ("slow".into(), Some(180)),
            ("fast".into(), Some(20)),
            ("timeout-a".into(), None),
        ];
        sort_node_delays(&mut results);
        assert_eq!(
            results,
            vec![
                ("fast".into(), Some(20)),
                ("slow".into(), Some(180)),
                ("timeout-a".into(), None),
                ("timeout-b".into(), None),
            ]
        );
    }

    #[test]
    fn readiness_requires_every_declared_proxy_port() {
        let mixed = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut runtime = RuntimeConfig::default();
        runtime.proxy.mixed_port = mixed.local_addr().unwrap().port();
        let mut capabilities = proxy_capabilities();
        assert!(proxy_ports_ready(&runtime, &capabilities));

        capabilities.http_proxy = true;
        runtime.proxy.http_port = 0;
        assert!(!proxy_ports_ready(&runtime, &capabilities));
    }

    #[test]
    fn parses_controller_delay_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 2048];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.starts_with("GET /proxies/HK%20Node/delay?"));
            assert!(request.contains("timeout=800"));
            let body = r#"{"delay":42}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let mut runtime = RuntimeConfig::default();
        runtime.api.port = port;
        assert_eq!(
            test_node_delay(
                &runtime,
                "HK Node",
                "https://www.gstatic.com/generate_204",
                800
            )
            .unwrap(),
            42
        );
        server.join().unwrap();
    }
}
