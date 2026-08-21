use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

const SCHEMA_VERSION: u32 = 1;

/// active.toml：主页开关 + 当前 core 选择。
/// profile 选择由 profiles.toml 维护；PID 等实时状态走 state/runtime/。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveConfig {
    pub schema_version: u32,
    #[serde(default)]
    pub current: Current,
    #[serde(default)]
    pub tun: Tun,
    #[serde(default)]
    pub shell_proxy: ShellProxy,
    #[serde(default)]
    pub system_proxy: SystemProxy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Current {
    /// 当前使用 core 的注册名（= cores 目录名）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub core: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Tun {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellProxy {
    pub enabled: bool,
    pub bypass: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemProxy {
    pub enabled: bool,
    pub bypass: bool,
}

impl Default for ActiveConfig {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            current: Current::default(),
            tun: Tun::default(),
            shell_proxy: ShellProxy::default(),
            system_proxy: SystemProxy::default(),
        }
    }
}

impl Default for ShellProxy {
    fn default() -> Self {
        Self {
            enabled: false,
            bypass: true,
        }
    }
}

impl Default for SystemProxy {
    fn default() -> Self {
        Self {
            enabled: false,
            bypass: true,
        }
    }
}

impl ActiveConfig {
    pub fn load(path: &Path) -> Result<Self, io::Error> {
        let content = fs::read_to_string(path)?;
        let active: Self = toml::from_str(&content).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid active.toml: {error}"),
            )
        })?;
        if active.schema_version != SCHEMA_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported active.toml schema_version {}; expected {SCHEMA_VERSION}",
                    active.schema_version
                ),
            ));
        }
        Ok(active)
    }

    pub fn save(&self, path: &Path) -> Result<(), io::Error> {
        let content = toml::to_string_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        crate::platform::atomic_write(path, content.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::ActiveConfig;

    #[test]
    fn default_active_contains_only_homepage_state() {
        let text = toml::to_string_pretty(&ActiveConfig::default()).expect("serialize");
        assert!(text.contains("[current]"));
        assert!(text.contains("[tun]"));
        assert!(text.contains("[shell_proxy]"));
        assert!(text.contains("[system_proxy]"));
        assert!(!text.contains("profile"));
        assert!(!text.contains("stack"));
        assert!(!text.contains("auto_route"));
    }
}
