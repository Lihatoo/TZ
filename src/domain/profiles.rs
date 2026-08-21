use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs, io,
    path::{Component, Path},
};

const SCHEMA_VERSION: u32 = 1;

/// data/profiles/profiles.toml：集中式 profile 索引、各 family 当前选择，
/// 以及每个 profile 的策略组节点选择。不会写回 source 文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfilesIndex {
    pub schema_version: u32,
    #[serde(default)]
    pub current: BTreeMap<String, String>,
    #[serde(default)]
    pub profiles: Vec<ProfileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProfileEntry {
    pub name: String,
    pub family: String,
    pub format: String,
    /// 相对 profiles_dir 的路径，例如 "home/source.yaml"。
    pub source_file: String,
    #[serde(default)]
    pub origin: ProfileOrigin,
    #[serde(default)]
    pub update: ProfileUpdate,
    #[serde(default)]
    pub state: ProfileState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileOrigin {
    /// "remote" | "local"
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub original_path: String,
    /// Route used to obtain a remote source: direct, proxy, or unknown.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub download_via: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProfileUpdate {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub updated_at: String,
}

/// builder 生成配置时，把策略组及其默认节点选择写入生成配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProfileState {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub selected: BTreeMap<String, String>,
}

impl Default for ProfilesIndex {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            current: BTreeMap::new(),
            profiles: Vec::new(),
        }
    }
}

impl Default for ProfileOrigin {
    fn default() -> Self {
        Self {
            kind: "local".into(),
            url: String::new(),
            original_path: String::new(),
            download_via: String::new(),
        }
    }
}

impl ProfilesIndex {
    pub fn load(path: &Path) -> Result<Self, io::Error> {
        let content = fs::read_to_string(path)?;
        let index: Self = toml::from_str(&content).map_err(|error| invalid(error.to_string()))?;
        index.validate()?;
        Ok(index)
    }

    pub fn save(&self, path: &Path) -> Result<(), io::Error> {
        self.validate()?;
        let content = toml::to_string_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        crate::platform::atomic_write_private(path, content.as_bytes())
    }

    pub fn find(&self, name: &str) -> Option<&ProfileEntry> {
        self.profiles.iter().find(|profile| profile.name == name)
    }

    fn validate(&self) -> Result<(), io::Error> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(invalid(format!(
                "unsupported schema_version {}; expected {SCHEMA_VERSION}",
                self.schema_version
            )));
        }

        let mut names = HashSet::new();
        for profile in &self.profiles {
            validate_name(&profile.name)?;
            validate_family_format(&profile.family, &profile.format)?;
            validate_relative_path(&profile.source_file, "source_file")?;
            validate_origin(&profile.origin)?;
            if !names.insert(profile.name.as_str()) {
                return Err(invalid(format!(
                    "duplicate profile name `{}`; names must be unique across families",
                    profile.name
                )));
            }
            for (group, node) in &profile.state.selected {
                if group.trim().is_empty() || node.trim().is_empty() {
                    return Err(invalid(format!(
                        "profile `{}` has an empty group or node selection",
                        profile.name
                    )));
                }
            }
        }

        for (family, name) in &self.current {
            validate_family(family)?;
            if !self
                .profiles
                .iter()
                .any(|profile| profile.family == *family && profile.name == *name)
            {
                return Err(invalid(format!(
                    "current profile `{name}` does not exist for family `{family}`"
                )));
            }
        }
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<(), io::Error> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(invalid(format!(
            "profile name `{name}` must contain only ASCII letters, numbers, '.', '_' or '-'"
        )));
    }
    Ok(())
}

fn validate_family(family: &str) -> Result<(), io::Error> {
    if !matches!(family, "clash" | "sing-box") {
        return Err(invalid(format!("unsupported profile family `{family}`")));
    }
    Ok(())
}

fn validate_family_format(family: &str, format: &str) -> Result<(), io::Error> {
    validate_family(family)?;
    if !matches!((family, format), ("clash", "yaml") | ("sing-box", "json")) {
        return Err(invalid(format!(
            "profile family `{family}` does not support format `{format}`"
        )));
    }
    Ok(())
}

fn validate_relative_path(value: &str, field: &str) -> Result<(), io::Error> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(invalid(format!(
            "{field} must be a non-empty relative path without '.' or '..': `{value}`"
        )));
    }
    Ok(())
}

fn validate_origin(origin: &ProfileOrigin) -> Result<(), io::Error> {
    match origin.kind.as_str() {
        "remote" if origin.url.starts_with("https://") || origin.url.starts_with("http://") => {
            if !origin.original_path.is_empty() {
                return Err(invalid(
                    "remote profile origin must not set original_path".into(),
                ));
            }
            if !origin.download_via.is_empty()
                && !matches!(origin.download_via.as_str(), "direct" | "proxy" | "unknown")
            {
                return Err(invalid(
                    "remote profile origin has invalid download_via".into(),
                ));
            }
        }
        "local"
            if !origin.original_path.is_empty()
                && Path::new(&origin.original_path).is_absolute() =>
        {
            if !origin.url.is_empty() {
                return Err(invalid("local profile origin must not set url".into()));
            }
        }
        "remote" => {
            return Err(invalid(
                "remote profile origin requires an HTTP(S) url".into(),
            ));
        }
        "local" => {
            return Err(invalid(
                "local profile origin requires an absolute original_path".into(),
            ));
        }
        kind => return Err(invalid(format!("unsupported profile origin kind `{kind}`"))),
    }
    Ok(())
}

fn invalid(message: String) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("invalid profiles.toml: {message}"),
    )
}

#[cfg(test)]
mod tests {
    use super::{ProfileEntry, ProfileOrigin, ProfilesIndex};

    fn local_profile(name: &str) -> ProfileEntry {
        ProfileEntry {
            name: name.into(),
            family: "clash".into(),
            format: "yaml".into(),
            source_file: format!("{name}/source.yaml"),
            origin: ProfileOrigin {
                kind: "local".into(),
                url: String::new(),
                original_path: format!("/tmp/{name}.yaml"),
                download_via: String::new(),
            },
            ..Default::default()
        }
    }

    #[test]
    fn profiles_index_defaults_to_empty_list() {
        let index = ProfilesIndex::default();
        assert!(index.profiles.is_empty());
        assert!(index.current.is_empty());
        assert_eq!(index.schema_version, 1);
    }

    #[test]
    fn profile_entry_serializes_current_and_group_selections() {
        let mut index = ProfilesIndex::default();
        let mut profile = local_profile("home");
        profile
            .state
            .selected
            .insert("Proxy".into(), "Hong Kong 01".into());
        index.current.insert("clash".into(), "home".into());
        index.profiles.push(profile);
        let text = toml::to_string_pretty(&index).expect("serialize");
        assert!(text.contains("[current]"));
        assert!(text.contains("[[profiles]]"));
        assert!(text.contains("[profiles.state.selected]"));
    }

    #[test]
    fn rejects_duplicate_profiles_and_unsafe_sources() {
        let mut index = ProfilesIndex::default();
        index.profiles.push(local_profile("home"));
        index.profiles.push(local_profile("home"));
        assert!(index.validate().is_err());

        let mut index = ProfilesIndex::default();
        index.profiles.push(local_profile("home"));
        let mut sing_box = local_profile("home");
        sing_box.family = "sing-box".into();
        sing_box.format = "json".into();
        sing_box.source_file = "home/source.json".into();
        index.profiles.push(sing_box);
        assert!(index.validate().is_err());

        let mut index = ProfilesIndex::default();
        let mut profile = local_profile("home");
        profile.source_file = "../outside.yaml".into();
        index.profiles.push(profile);
        assert!(index.validate().is_err());
    }

    #[test]
    fn rejects_missing_current_profile() {
        let mut index = ProfilesIndex::default();
        index.current.insert("clash".into(), "missing".into());
        assert!(index.validate().is_err());
    }
}
