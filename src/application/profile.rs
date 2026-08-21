use std::{error::Error, fmt, fs, io, os::unix::fs::PermissionsExt, path::Path};

use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;

use crate::{
    domain::{ProfileEntry, ProfileOrigin, ProfileUpdate, ProfilesIndex},
    platform::{AppLock, AppPaths, ProfileSource, atomic_write_private, ensure_not_running},
};

#[derive(Debug)]
pub enum ProfileError {
    InvalidInput(String),
    NotFound(String),
    AlreadyExists(String),
    Unsupported(String),
    Io(io::Error),
    Source(Box<dyn Error + Send + Sync>),
}

impl fmt::Display for ProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(message) => write!(formatter, "invalid profile: {message}"),
            Self::NotFound(name) => write!(formatter, "profile `{name}` does not exist"),
            Self::AlreadyExists(name) => write!(formatter, "profile `{name}` already exists"),
            Self::Unsupported(message) => formatter.write_str(message),
            Self::Io(error) => write!(formatter, "profile operation failed: {error}"),
            Self::Source(error) => write!(formatter, "profile download failed: {error}"),
        }
    }
}

impl Error for ProfileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Source(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

impl From<io::Error> for ProfileError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddProfile<'a> {
    pub name: &'a str,
    pub family: &'a str,
    pub source: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileSummary {
    pub name: String,
    pub family: String,
    pub format: String,
    pub current: bool,
}

pub struct ProfileService<'a, D> {
    paths: &'a AppPaths,
    downloader: &'a D,
}

impl<'a, D: ProfileSource> ProfileService<'a, D> {
    pub fn new(paths: &'a AppPaths, downloader: &'a D) -> Self {
        Self { paths, downloader }
    }

    pub fn add(&self, request: AddProfile<'_>) -> Result<ProfileEntry, ProfileError> {
        validate_name(request.name)?;
        let format = format_for_family(request.family)?;
        let (content, origin) = if is_http_url(request.source) {
            let (content, download_via) = self
                .downloader
                .download_with_route(request.source)
                .map_err(|error| ProfileError::Source(Box::new(error)))?;
            (
                content,
                ProfileOrigin {
                    kind: "remote".into(),
                    url: request.source.into(),
                    original_path: String::new(),
                    download_via: download_via.as_str().into(),
                },
            )
        } else if request.source.contains("://") {
            return Err(ProfileError::InvalidInput(
                "remote source must use http:// or https://".into(),
            ));
        } else {
            read_local_source(request.source)?
        };
        validate_content(request.family, &content)?;

        let _lock = AppLock::acquire(&self.paths.lock_file())?;
        fs::create_dir_all(self.paths.profiles_dir())?;
        fs::set_permissions(self.paths.profiles_dir(), fs::Permissions::from_mode(0o700))?;
        let mut index = self.load_index()?;
        if index
            .profiles
            .iter()
            .any(|profile| profile.name == request.name)
        {
            return Err(ProfileError::AlreadyExists(request.name.into()));
        }
        let entry = ProfileEntry {
            name: request.name.into(),
            family: request.family.into(),
            format: format.into(),
            source_file: managed_relative_path(request.name, format),
            origin,
            update: ProfileUpdate::default(),
            state: Default::default(),
        };
        let source_path = self.paths.profiles_dir().join(&entry.source_file);
        commit_source_and_index(
            &source_path,
            &content,
            &mut index,
            |index| {
                index.profiles.push(entry.clone());
            },
            &self.paths.profiles_file(),
        )?;
        invalidate_generated(self.paths)?;
        Ok(entry)
    }

    pub fn list(&self, family: Option<&str>) -> Result<Vec<ProfileSummary>, ProfileError> {
        if let Some(family) = family {
            format_for_family(family)?;
        }
        let index = self.load_index()?;
        let mut profiles: Vec<_> = index
            .profiles
            .iter()
            .filter(|profile| family.is_none_or(|family| profile.family == family))
            .map(|profile| ProfileSummary {
                name: profile.name.clone(),
                family: profile.family.clone(),
                format: profile.format.clone(),
                current: index.current.get(&profile.family) == Some(&profile.name),
            })
            .collect();
        profiles.sort_by(|left, right| {
            left.family
                .cmp(&right.family)
                .then(left.name.cmp(&right.name))
        });
        Ok(profiles)
    }

    pub fn info(&self, name: &str) -> Result<ProfileEntry, ProfileError> {
        self.load_index()?
            .profiles
            .into_iter()
            .find(|profile| profile.name == name)
            .ok_or_else(|| ProfileError::NotFound(name.into()))
    }

    pub fn use_profile(&self, name: &str) -> Result<ProfileEntry, ProfileError> {
        let _lock = AppLock::acquire(&self.paths.lock_file())?;
        ensure_not_running(&self.paths.core_pid_file())?;
        let mut index = self.load_index()?;
        let profile = index
            .profiles
            .iter()
            .find(|profile| profile.name == name)
            .cloned()
            .ok_or_else(|| ProfileError::NotFound(name.into()))?;
        index
            .current
            .insert(profile.family.clone(), profile.name.clone());
        save_index(&index, &self.paths.profiles_file())?;
        invalidate_generated(self.paths)?;
        Ok(profile)
    }

    pub fn update(&self, name: &str) -> Result<ProfileEntry, ProfileError> {
        let snapshot = self.info(name)?;
        if snapshot.origin.kind != "remote" {
            return Err(ProfileError::Unsupported(format!(
                "local profile `{name}` cannot be updated"
            )));
        }
        let (content, download_via) = self
            .downloader
            .download_with_route(&snapshot.origin.url)
            .map_err(|error| ProfileError::Source(Box::new(error)))?;
        validate_content(&snapshot.family, &content)?;

        let _lock = AppLock::acquire(&self.paths.lock_file())?;
        ensure_not_running(&self.paths.core_pid_file())?;
        let mut index = self.load_index()?;
        let position = index
            .profiles
            .iter()
            .position(|profile| profile.name == name)
            .ok_or_else(|| ProfileError::NotFound(name.into()))?;
        if index.profiles[position].origin.url != snapshot.origin.url {
            return Err(ProfileError::InvalidInput(format!(
                "profile `{name}` changed while it was being downloaded"
            )));
        }
        index.profiles[position].origin.download_via = download_via.as_str().into();
        let source_path = self
            .paths
            .profiles_dir()
            .join(&index.profiles[position].source_file);
        let previous = fs::read(&source_path)?;
        atomic_write_private(&source_path, &content)?;
        index.profiles[position].update.updated_at = jiff::Timestamp::now().to_string();
        if let Err(error) = save_index(&index, &self.paths.profiles_file()) {
            let _ = atomic_write_private(&source_path, &previous);
            return Err(error);
        }
        invalidate_generated(self.paths)?;
        Ok(index.profiles[position].clone())
    }

    pub fn remove(&self, name: &str) -> Result<ProfileEntry, ProfileError> {
        let _lock = AppLock::acquire(&self.paths.lock_file())?;
        ensure_not_running(&self.paths.core_pid_file())?;
        let mut index = self.load_index()?;
        let position = index
            .profiles
            .iter()
            .position(|profile| profile.name == name)
            .ok_or_else(|| ProfileError::NotFound(name.into()))?;
        let removed = index.profiles.remove(position);
        if index.current.get(&removed.family) == Some(&removed.name) {
            index.current.remove(&removed.family);
        }

        let profile_dir = self.paths.profiles_dir().join(&removed.name);
        let tombstone = self.paths.profiles_dir().join(format!(
            ".removing-{}-{}",
            removed.name,
            std::process::id()
        ));
        if profile_dir.exists() {
            fs::rename(&profile_dir, &tombstone)?;
        }
        if let Err(error) = save_index(&index, &self.paths.profiles_file()) {
            if tombstone.exists() {
                let _ = fs::rename(&tombstone, &profile_dir);
            }
            return Err(error);
        }
        if tombstone.exists() {
            fs::remove_dir_all(&tombstone)?;
        }
        invalidate_generated(self.paths)?;
        Ok(removed)
    }

    fn load_index(&self) -> Result<ProfilesIndex, ProfileError> {
        if self.paths.profiles_file().is_file() {
            Ok(ProfilesIndex::load(&self.paths.profiles_file())?)
        } else {
            Ok(ProfilesIndex::default())
        }
    }
}

fn invalidate_generated(paths: &AppPaths) -> Result<(), ProfileError> {
    if paths.generated_dir().is_dir() {
        fs::remove_dir_all(paths.generated_dir())?;
    }
    fs::create_dir_all(paths.generated_dir())?;
    Ok(())
}

fn read_local_source(value: &str) -> Result<(Vec<u8>, ProfileOrigin), ProfileError> {
    let path = Path::new(value);
    let canonical = fs::canonicalize(path)?;
    if !canonical.metadata()?.is_file() {
        return Err(ProfileError::InvalidInput(format!(
            "local source is not a regular file: {}",
            canonical.display()
        )));
    }
    let content = fs::read(&canonical)?;
    Ok((
        content,
        ProfileOrigin {
            kind: "local".into(),
            url: String::new(),
            original_path: canonical.to_string_lossy().into_owned(),
            download_via: String::new(),
        },
    ))
}

fn commit_source_and_index(
    source_path: &Path,
    content: &[u8],
    index: &mut ProfilesIndex,
    update: impl FnOnce(&mut ProfilesIndex),
    index_path: &Path,
) -> Result<(), ProfileError> {
    atomic_write_private(source_path, content)?;
    update(index);
    if let Err(error) = save_index(index, index_path) {
        let _ = fs::remove_file(source_path);
        return Err(error);
    }
    Ok(())
}

fn save_index(index: &ProfilesIndex, path: &Path) -> Result<(), ProfileError> {
    let content = toml::to_string_pretty(index)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    atomic_write_private(path, content.as_bytes())?;
    Ok(())
}

fn validate_name(name: &str) -> Result<(), ProfileError> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ProfileError::InvalidInput(
            "name must contain only ASCII letters, numbers, '.', '_' or '-'".into(),
        ));
    }
    Ok(())
}

fn format_for_family(family: &str) -> Result<&'static str, ProfileError> {
    match family {
        "clash" => Ok("yaml"),
        "sing-box" => Ok("json"),
        _ => Err(ProfileError::InvalidInput(format!(
            "unsupported family `{family}`"
        ))),
    }
}

fn managed_relative_path(name: &str, format: &str) -> String {
    format!("{name}/source.{format}")
}

fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn validate_content(family: &str, content: &[u8]) -> Result<(), ProfileError> {
    match family {
        "clash" => validate_clash(content),
        "sing-box" => validate_sing_box(content),
        _ => Err(ProfileError::InvalidInput(format!(
            "unsupported family `{family}`"
        ))),
    }
}

fn validate_clash(content: &[u8]) -> Result<(), ProfileError> {
    let value: YamlValue = serde_yaml::from_slice(content)
        .map_err(|error| ProfileError::InvalidInput(format!("invalid Clash YAML: {error}")))?;
    let proxies = value
        .as_mapping()
        .and_then(|mapping| mapping.get(YamlValue::String("proxies".into())))
        .and_then(YamlValue::as_sequence);
    if proxies.is_none_or(Vec::is_empty) {
        return Err(ProfileError::InvalidInput(
            "Clash YAML must contain at least one proxy".into(),
        ));
    }
    Ok(())
}

fn validate_sing_box(content: &[u8]) -> Result<(), ProfileError> {
    let value: JsonValue = serde_json::from_slice(content)
        .map_err(|error| ProfileError::InvalidInput(format!("invalid sing-box JSON: {error}")))?;
    let outbounds = value.get("outbounds").and_then(JsonValue::as_array);
    if outbounds.is_none_or(Vec::is_empty) {
        return Err(ProfileError::InvalidInput(
            "sing-box JSON must contain at least one outbound".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AddProfile, ProfileError, ProfileService};
    use crate::{
        domain::ProfilesIndex,
        platform::{AppPaths, DownloadError, ProfileSource},
    };
    use std::{cell::RefCell, fs};
    use tempfile::tempdir;

    struct FakeSource {
        response: RefCell<Result<Vec<u8>, DownloadError>>,
    }

    impl FakeSource {
        fn success(content: &[u8]) -> Self {
            Self {
                response: RefCell::new(Ok(content.to_vec())),
            }
        }
    }

    impl ProfileSource for FakeSource {
        fn download(&self, _url: &str) -> Result<Vec<u8>, DownloadError> {
            self.response.borrow().clone()
        }
    }

    fn initialized_paths(root: &std::path::Path) -> AppPaths {
        let paths = AppPaths::unified(root);
        paths.ensure_dirs().unwrap();
        ProfilesIndex::default()
            .save(&paths.profiles_file())
            .unwrap();
        paths
    }

    #[test]
    fn local_add_creates_private_managed_copy_without_touching_original() {
        let root = tempdir().unwrap();
        let paths = initialized_paths(root.path());
        let original = root.path().join("original.yaml");
        fs::write(&original, "proxies:\n  - name: node\n    type: direct\n").unwrap();
        let fake = FakeSource::success(b"unused");
        let service = ProfileService::new(&paths, &fake);

        let entry = service
            .add(AddProfile {
                name: "home",
                family: "clash",
                source: original.to_str().unwrap(),
            })
            .unwrap();
        assert_eq!(
            entry.origin.original_path,
            original.canonicalize().unwrap().to_string_lossy()
        );
        let managed = paths.profiles_dir().join(entry.source_file);
        assert_eq!(
            fs::read_to_string(&managed).unwrap(),
            fs::read_to_string(&original).unwrap()
        );
        fs::remove_file(managed).unwrap();
        assert!(original.is_file());
    }

    #[test]
    fn supports_family_specific_validation_and_list_use_info() {
        let root = tempdir().unwrap();
        let paths = initialized_paths(root.path());
        let fake = FakeSource::success(br#"{"outbounds":[{"type":"direct"}]}"#);
        let service = ProfileService::new(&paths, &fake);
        service
            .add(AddProfile {
                name: "remote",
                family: "sing-box",
                source: "https://example.com/sub?token=secret",
            })
            .unwrap();
        assert_eq!(service.info("remote").unwrap().format, "json");
        service.use_profile("remote").unwrap();
        let listed = service.list(Some("sing-box")).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].current);
    }

    #[test]
    fn rejects_duplicate_invalid_and_family_mismatched_content() {
        let root = tempdir().unwrap();
        let paths = initialized_paths(root.path());
        let fake = FakeSource::success(b"proxies:\n  - {name: node}\n");
        let service = ProfileService::new(&paths, &fake);
        let request = AddProfile {
            name: "home",
            family: "clash",
            source: "https://example.com/sub",
        };
        service.add(request.clone()).unwrap();
        assert!(matches!(
            service.add(request),
            Err(ProfileError::AlreadyExists(_))
        ));

        let bad = FakeSource::success(br#"{"outbounds":[]}"#);
        let service = ProfileService::new(&paths, &bad);
        assert!(matches!(
            service.add(AddProfile {
                name: "bad",
                family: "sing-box",
                source: "https://example.com/bad"
            }),
            Err(ProfileError::InvalidInput(_))
        ));
    }

    #[test]
    fn failed_update_preserves_previous_managed_copy() {
        let root = tempdir().unwrap();
        let paths = initialized_paths(root.path());
        let fake = FakeSource::success(b"proxies:\n  - {name: old}\n");
        let service = ProfileService::new(&paths, &fake);
        let entry = service
            .add(AddProfile {
                name: "remote",
                family: "clash",
                source: "https://example.com/sub",
            })
            .unwrap();
        let managed = paths.profiles_dir().join(&entry.source_file);
        *fake.response.borrow_mut() = Ok(b"proxies: []\n".to_vec());
        assert!(service.update("remote").is_err());
        assert_eq!(
            fs::read_to_string(managed).unwrap(),
            "proxies:\n  - {name: old}\n"
        );
    }

    #[test]
    fn remove_clears_current_and_only_deletes_managed_copy() {
        let root = tempdir().unwrap();
        let paths = initialized_paths(root.path());
        let original = root.path().join("original.yaml");
        fs::write(&original, "proxies:\n  - {name: node}\n").unwrap();
        let fake = FakeSource::success(b"unused");
        let service = ProfileService::new(&paths, &fake);
        let entry = service
            .add(AddProfile {
                name: "home",
                family: "clash",
                source: original.to_str().unwrap(),
            })
            .unwrap();
        service.use_profile("home").unwrap();
        service.remove("home").unwrap();
        assert!(original.is_file());
        assert!(!paths.profiles_dir().join(entry.source_file).exists());
        let index = ProfilesIndex::load(&paths.profiles_file()).unwrap();
        assert!(index.current.is_empty());
        assert!(index.profiles.is_empty());
    }
}
