use std::{
    env,
    error::Error,
    fmt, fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

const PATHS_FILE_ENV: &str = "TZ_PATHS_TOML";
const DEFAULT_PATHS_FILE: &str = ".config/tz/paths.toml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathsFile {
    pub layout: LayoutFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutFile {
    pub config_dir: String,
    pub data_dir: String,
    pub state_dir: String,
    pub cache_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
}

#[derive(Debug)]
pub enum PathError {
    HomeNotFound,
    Io(io::Error),
    InvalidPath(String),
    InvalidFile(String),
    NotInitialized { checked: Vec<PathBuf> },
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeNotFound => write!(f, "cannot determine user home directory"),
            Self::Io(error) => write!(f, "path operation failed: {error}"),
            Self::InvalidPath(error) => write!(f, "invalid path: {error}"),
            Self::InvalidFile(error) => write!(f, "invalid paths.toml: {error}"),
            Self::NotInitialized { checked } => {
                write!(f, "tz is not initialized; checked: ")?;
                for (index, path) in checked.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", path.display())?;
                }
                write!(f, "; run tz init or check TZ_PATHS_TOML")
            }
        }
    }
}

impl Error for PathError {}

impl From<io::Error> for PathError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn paths_file() -> Result<PathBuf, PathError> {
    if let Some(path) = env::var_os(PATHS_FILE_ENV).filter(|value| !value.is_empty()) {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(PathError::InvalidPath(format!(
                "{PATHS_FILE_ENV} must be an absolute path"
            )));
        }
        return Ok(path);
    }
    Ok(home_dir()?.join(DEFAULT_PATHS_FILE))
}

pub fn resolve_paths() -> Result<AppPaths, PathError> {
    let file = paths_file()?;
    if !file.is_file() {
        return Err(PathError::NotInitialized {
            checked: vec![file],
        });
    }
    load_paths_file(&file)
}

/// 加载路径，未初始化时返回错误而不是回退默认值。
/// cli::run 在调用前已先 resolve_paths 做过校验，因此这里通常不会报错。
pub fn load_or_fail() -> Result<AppPaths, PathError> {
    resolve_paths()
}

pub fn load_paths_file(file: &Path) -> Result<AppPaths, PathError> {
    let content = fs::read_to_string(file)?;
    let parsed: PathsFile =
        toml::from_str(&content).map_err(|error| PathError::InvalidFile(error.to_string()))?;
    AppPaths::from_layout_file(parsed.layout)
}

pub fn save_paths_file(file: &Path, paths: &AppPaths) -> Result<(), PathError> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let home = home_dir()?;
    let document = PathsFile {
        layout: LayoutFile {
            config_dir: display_path(&paths.config_dir, &home),
            data_dir: display_path(&paths.data_dir, &home),
            state_dir: display_path(&paths.state_dir, &home),
            cache_dir: display_path(&paths.cache_dir, &home),
        },
    };
    let content = toml::to_string_pretty(&document)
        .map_err(|error| PathError::InvalidFile(error.to_string()))?;
    fs::write(file, content)?;
    Ok(())
}

fn display_path(path: &Path, home: &Path) -> String {
    path.strip_prefix(home)
        .map(|relative| format!("~/{}", relative.display()))
        .unwrap_or_else(|_| path.display().to_string())
}

impl AppPaths {
    pub fn from_layout_file(layout: LayoutFile) -> Result<Self, PathError> {
        Ok(Self {
            config_dir: expand_path(&layout.config_dir)?,
            data_dir: expand_path(&layout.data_dir)?,
            state_dir: expand_path(&layout.state_dir)?,
            cache_dir: expand_path(&layout.cache_dir)?,
        })
    }

    pub fn from_layout(layout: LayoutFile) -> Result<Self, PathError> {
        Self::from_layout_file(layout)
    }

    /// 从当前进程环境加载路径配置。未初始化时返回错误。
    pub fn from_env_or_none() -> Result<Option<Self>, PathError> {
        match resolve_paths() {
            Ok(paths) => Ok(Some(paths)),
            Err(PathError::NotInitialized { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn unified(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            state_dir: root.join("state"),
            cache_dir: root.join("cache"),
        }
    }

    pub fn ensure_dirs(&self) -> io::Result<()> {
        for directory in [
            &self.config_dir,
            &self.data_dir,
            &self.state_dir,
            &self.cache_dir,
            &self.profiles_dir(),
            &self.cores_dir(),
            &self.generated_dir(),
            &self.runtime_dir(),
            &self.logs_dir(),
            &self.downloads_dir(),
            &self.speedtest_dir(),
        ] {
            fs::create_dir_all(directory)?;
        }
        Ok(())
    }

    pub fn is_initialized(&self) -> bool {
        self.settings_file().is_file()
    }

    pub fn initialize_files(&self) -> io::Result<()> {
        self.ensure_dirs()?;
        // bypass.list 没有 domain 结构体，直接写占位。
        write_if_missing(
            &self.bypass_file(),
            "# One domain, host, or CIDR per line.\n",
        )?;
        Ok(())
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }

    pub fn runtime_file(&self) -> PathBuf {
        self.config_dir.join("runtime.toml")
    }

    pub fn bypass_file(&self) -> PathBuf {
        self.config_dir.join("bypass.list")
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.data_dir.join("profiles")
    }

    pub fn profiles_file(&self) -> PathBuf {
        self.profiles_dir().join("profiles.toml")
    }

    pub fn cores_dir(&self) -> PathBuf {
        self.data_dir.join("cores")
    }

    pub fn active_file(&self) -> PathBuf {
        self.state_dir.join("active.toml")
    }

    pub fn generated_dir(&self) -> PathBuf {
        self.state_dir.join("generated")
    }

    /// state/runtime/<core>/：core 的工作目录（{workdir} 展开目标），
    /// 避免 cache.db 等运行副产物污染 generated/<core>/。
    pub fn core_workdir(&self, core_name: &str) -> PathBuf {
        self.runtime_dir().join(core_name)
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.state_dir.join("runtime")
    }

    pub fn core_pid_file(&self) -> PathBuf {
        self.runtime_dir().join("core.pid")
    }

    pub fn lock_file(&self) -> PathBuf {
        self.runtime_dir().join("tz.lock")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.state_dir.join("logs")
    }

    pub fn tz_log_file(&self) -> PathBuf {
        self.logs_dir().join("tz.log")
    }

    pub fn core_log_file(&self) -> PathBuf {
        self.logs_dir().join("core.log")
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.cache_dir.join("downloads")
    }

    pub fn speedtest_dir(&self) -> PathBuf {
        self.cache_dir.join("speedtest")
    }
}

fn expand_path(value: &str) -> Result<PathBuf, PathError> {
    let path = if value == "~" {
        home_dir()?
    } else if let Some(relative) = value.strip_prefix("~/") {
        home_dir()?.join(relative)
    } else {
        PathBuf::from(value)
    };
    if !path.is_absolute() {
        return Err(PathError::InvalidPath(format!(
            "path must be absolute or start with ~/ (got {value})"
        )));
    }
    Ok(path)
}

fn home_dir() -> Result<PathBuf, PathError> {
    env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or(PathError::HomeNotFound)
}

fn write_if_missing(path: &Path, content: &str) -> io::Result<()> {
    if !path.exists() {
        fs::write(path, content)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AppPaths, LayoutFile, load_paths_file, save_paths_file};
    use std::path::PathBuf;

    #[test]
    fn paths_file_expands_home_prefix() {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .expect("test HOME should be set");
        let file = unique_temp_path("tz-paths");
        std::fs::write(
            &file,
            "[layout]\nconfig_dir = \"~/config\"\ndata_dir = \"/tmp/data\"\nstate_dir = \"/tmp/state\"\ncache_dir = \"/tmp/cache\"\n",
        )
        .expect("write paths fixture");
        let paths = load_paths_file(&file).expect("paths should load");
        assert_eq!(paths.config_dir, home.join("config"));
        std::fs::remove_file(file).expect("remove paths fixture");
    }

    #[test]
    fn paths_file_round_trips() {
        let root = unique_temp_path("tz-roundtrip");
        let paths = AppPaths::unified(&root);
        let file = root.join("paths.toml");
        save_paths_file(&file, &paths).expect("paths should save");
        let loaded = load_paths_file(&file).expect("paths should load");
        assert_eq!(loaded, paths);
        std::fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn missing_layout_field_is_rejected() {
        let file = unique_temp_path("tz-invalid-paths");
        std::fs::write(&file, "[layout]\nconfig_dir = \"/tmp/config\"\n")
            .expect("write invalid fixture");
        assert!(load_paths_file(&file).is_err());
        std::fs::remove_file(file).expect("remove invalid fixture");
    }

    #[test]
    fn layout_file_has_four_fixed_fields() {
        let layout = LayoutFile {
            config_dir: "/tmp/config".into(),
            data_dir: "/tmp/data".into(),
            state_dir: "/tmp/state".into(),
            cache_dir: "/tmp/cache".into(),
        };
        let paths = AppPaths::from_layout(layout).expect("layout should load");
        assert_eq!(paths.cache_dir, PathBuf::from("/tmp/cache"));
    }

    fn unique_temp_path(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}", std::process::id()))
    }
}
