use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

const SCHEMA_VERSION: u32 = 1;

/// runtime.toml：跨 core 都能表达的运行参数层（端口、API、DNS、TUN 参数）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub schema_version: u32,
    #[serde(default)]
    pub proxy: ProxyConfig,
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub dns: DnsConfig,
    #[serde(default)]
    pub tun: TunConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyConfig {
    pub mode: String,
    pub listen: String,
    pub mixed_port: u16,
    pub http_port: u16,
    pub socks_port: u16,
    pub allow_lan: bool,
    pub ipv6: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiConfig {
    pub enabled: bool,
    pub listen: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DnsConfig {
    pub enabled: bool,
    pub listen: String,
    pub port: u16,
    pub ipv6: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunConfig {
    pub stack: String,
    pub auto_route: bool,
    pub auto_detect_interface: bool,
    pub dns_hijack: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            proxy: ProxyConfig::default(),
            api: ApiConfig::default(),
            dns: DnsConfig::default(),
            tun: TunConfig::default(),
        }
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            mode: "rule".into(),
            listen: "127.0.0.1".into(),
            mixed_port: 7890,
            http_port: 7892,
            socks_port: 7891,
            allow_lan: false,
            ipv6: false,
        }
    }
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            listen: "127.0.0.1".into(),
            port: 9189,
        }
    }
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            listen: "127.0.0.1".into(),
            port: 1053,
            ipv6: false,
        }
    }
}

impl Default for TunConfig {
    fn default() -> Self {
        Self {
            stack: "system".into(),
            auto_route: true,
            auto_detect_interface: true,
            dns_hijack: true,
        }
    }
}

impl RuntimeConfig {
    pub fn load(path: &Path) -> Result<Self, io::Error> {
        let content = fs::read_to_string(path)?;
        let runtime: Self = toml::from_str(&content).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid runtime.toml: {error}"),
            )
        })?;
        if runtime.schema_version != SCHEMA_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported runtime.toml schema_version {}; expected {SCHEMA_VERSION}",
                    runtime.schema_version
                ),
            ));
        }
        Ok(runtime)
    }

    pub fn save(&self, path: &Path) -> Result<(), io::Error> {
        let content = toml::to_string_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        crate::platform::atomic_write(path, content.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeConfig;

    #[test]
    fn default_runtime_matches_doc() {
        let text = toml::to_string_pretty(&RuntimeConfig::default()).expect("serialize");
        assert!(text.contains("[proxy]"));
        assert!(text.contains("mixed_port = 7890"));
        assert!(text.contains("http_port = 7892"));
        assert!(text.contains("socks_port = 7891"));
        assert!(text.contains("[api]"));
        assert!(text.contains("port = 9189"));
        assert!(text.contains("[dns]"));
        assert!(text.contains("[tun]"));
    }
}
