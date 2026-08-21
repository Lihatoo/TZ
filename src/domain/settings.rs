use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

const SCHEMA_VERSION: u32 = 1;

/// settings.toml：tz 软件自身的全局策略，与 core 运行关系不大。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub schema_version: u32,
    #[serde(default)]
    pub bypass: BypassConfig,
    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub update: UpdateConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BypassConfig {
    pub enabled: bool,
    /// 除 bypass.list 外，直接内联的补充条目。
    pub inline: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogConfig {
    pub level: String,
    /// tz.log 超过该大小直接清除重建，不保留归档。
    pub max_size_mb: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConfig {
    pub profiles: UpdateSection,
    pub cores: UpdateSection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateSection {
    pub auto_update: bool,
    pub interval_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            bypass: BypassConfig::default(),
            log: LogConfig::default(),
            update: UpdateConfig::default(),
        }
    }
}

impl Default for BypassConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            inline: vec!["localhost".into(), "127.0.0.0/8".into()],
        }
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "warn".into(),
            max_size_mb: 10,
        }
    }
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            profiles: UpdateSection {
                auto_update: false,
                interval_minutes: 4320,
            },
            cores: UpdateSection {
                auto_update: false,
                interval_minutes: 14400,
            },
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self, io::Error> {
        let content = fs::read_to_string(path)?;
        let settings: Self = toml::from_str(&content).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid settings.toml: {error}"),
            )
        })?;
        if settings.schema_version != SCHEMA_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported settings.toml schema_version {}; expected {SCHEMA_VERSION}",
                    settings.schema_version
                ),
            ));
        }
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> Result<(), io::Error> {
        let content = toml::to_string_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        crate::platform::atomic_write(path, content.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn default_settings_write_log_and_bypass() {
        let settings = Settings::default();
        let text = toml::to_string_pretty(&settings).expect("serialize");
        assert!(text.contains("[bypass]"));
        assert!(text.contains("max_size_mb = 10"));
        assert!(!text.contains("keep"));
    }

    #[test]
    fn default_settings_contain_update_sections() {
        let settings = Settings::default();
        let text = toml::to_string_pretty(&settings).expect("serialize");
        assert!(text.contains("[update.profiles]"));
        assert!(text.contains("[update.cores]"));
    }

    #[test]
    fn rejects_unknown_fields_and_schema_versions() {
        let file = std::env::temp_dir().join(format!("tz-settings-{}", std::process::id()));
        let mut text = toml::to_string_pretty(&Settings::default()).unwrap();
        text.insert_str(text.find("[bypass]").unwrap(), "unknown = true\n\n");
        std::fs::write(&file, text).unwrap();
        assert!(Settings::load(&file).is_err());

        let settings = Settings {
            schema_version: 2,
            ..Default::default()
        };
        std::fs::write(&file, toml::to_string(&settings).unwrap()).unwrap();
        assert!(Settings::load(&file).is_err());
        std::fs::remove_file(file).unwrap();
    }
}
