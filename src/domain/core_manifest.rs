use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{Component, Path, PathBuf},
};

const SCHEMA_VERSION: u32 = 1;

/// data/cores/<name>/core.toml：手动制作、tz 只读。
/// core.name 必须和目录名一致；不一致直接报错。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreManifest {
    pub schema_version: u32,
    pub core: CoreSection,
    pub runtime: RuntimeSection,
    pub capabilities: Capabilities,
    pub commands: Commands,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreSection {
    pub name: String,
    pub family: String,
    pub version: String,
    /// 相对 core 目录的二进制文件名。
    pub binary: String,
    pub os: String,
    pub arch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSection {
    /// 生成配置目录内的入口文件名（例如 config.yaml / config.json）。
    pub entrypoint: String,
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub config: ConfigCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigCapabilities {
    pub mixed_proxy: bool,
    pub http_proxy: bool,
    pub socks_proxy: bool,
    pub api: bool,
    pub dns: bool,
    pub tun: bool,
}

/// start 必填；check/version/reload 是否存在就是对应 CLI 动作的能力来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Commands {
    pub start: CommandArgs,
    #[serde(default)]
    pub check: Option<CommandArgs>,
    #[serde(default)]
    pub version: Option<CommandArgs>,
    #[serde(default)]
    pub reload: Option<CommandArgs>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandArgs {
    #[serde(default)]
    pub args: Vec<String>,
}

/// 已加载的 core，解析后带目录路径，可直接拼 spawn 命令。
#[derive(Debug, Clone)]
pub struct CoreDescriptor {
    /// core 注册名 = 目录名（mihomo、sing-box 等）。
    pub name: String,
    pub dir: PathBuf,
    pub manifest: CoreManifest,
}

impl CoreDescriptor {
    pub fn binary_path(&self) -> PathBuf {
        self.dir.join(&self.manifest.core.binary)
    }

    pub fn entrypoint_name(&self) -> &str {
        &self.manifest.runtime.entrypoint
    }

    /// 将 {config} {workdir} 占位符替换为真实路径。
    pub fn render_args(&self, args: &[String], config: &Path, workdir: &Path) -> Vec<String> {
        let config = config.display().to_string();
        let workdir = workdir.display().to_string();
        args.iter()
            .map(|arg| {
                arg.replace("{config}", &config)
                    .replace("{workdir}", &workdir)
            })
            .collect()
    }
}

pub fn load_manifest(dir: &Path) -> Result<CoreManifest, io::Error> {
    load_manifest_impl(dir, true)
}

/// 加载待导入的 core 包。来源目录名可以与稳定注册名不同；包内容仍执行完整校验。
pub fn load_import_manifest(dir: &Path) -> Result<CoreManifest, io::Error> {
    load_manifest_impl(dir, false)
}

fn load_manifest_impl(dir: &Path, require_directory_name: bool) -> Result<CoreManifest, io::Error> {
    let path = dir.join("core.toml");
    let content = fs::read_to_string(&path)?;
    let manifest: CoreManifest = toml::from_str(&content).map_err(|error| {
        invalid(format!(
            "cannot parse core.toml at {}: {error}",
            path.display()
        ))
    })?;
    validate_manifest(dir, &manifest, require_directory_name)?;
    Ok(manifest)
}

pub fn list_cores(cores_dir: &Path) -> Result<Vec<CoreDescriptor>, io::Error> {
    if !cores_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(cores_dir)? {
        let entry = entry?;
        let dir = entry.path();
        if !dir.is_dir() || !dir.join("core.toml").is_file() {
            continue;
        }
        let manifest = load_manifest(&dir)?;
        let name = dir_name(&dir)?.to_owned();
        entries.push(CoreDescriptor {
            name,
            dir,
            manifest,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

fn validate_manifest(
    dir: &Path,
    manifest: &CoreManifest,
    require_directory_name: bool,
) -> Result<(), io::Error> {
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(invalid(format!(
            "unsupported schema_version {}; expected {SCHEMA_VERSION}",
            manifest.schema_version
        )));
    }

    validate_name(&manifest.core.name)?;
    if require_directory_name {
        let directory_name = dir_name(dir)?;
        if manifest.core.name != directory_name {
            return Err(invalid(format!(
                "core name mismatch: dir is `{directory_name}` but core.toml declares `{}`",
                manifest.core.name
            )));
        }
    }
    if manifest.core.version.trim().is_empty() {
        return Err(invalid("core.version must not be empty".into()));
    }
    validate_platform(&manifest.core.os, &manifest.core.arch)?;
    validate_family_format(&manifest.core.family, &manifest.runtime.format)?;
    validate_file_name(&manifest.core.binary, "core.binary")?;
    validate_file_name(&manifest.runtime.entrypoint, "runtime.entrypoint")?;

    let binary = dir.join(&manifest.core.binary);
    let metadata = fs::metadata(&binary).map_err(|error| {
        invalid(format!(
            "core binary {} is not accessible: {error}",
            binary.display()
        ))
    })?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(invalid(format!(
            "core binary {} must be an executable file",
            binary.display()
        )));
    }

    validate_command("commands.start", &manifest.commands.start)?;
    for (name, command) in [
        ("commands.check", manifest.commands.check.as_ref()),
        ("commands.version", manifest.commands.version.as_ref()),
        ("commands.reload", manifest.commands.reload.as_ref()),
    ] {
        if let Some(command) = command {
            validate_command(name, command)?;
        }
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), io::Error> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(invalid(format!(
            "core.name `{name}` must contain only ASCII letters, numbers, '.', '_' or '-'"
        )));
    }
    Ok(())
}

fn validate_platform(os: &str, arch: &str) -> Result<(), io::Error> {
    if os != std::env::consts::OS || arch != std::env::consts::ARCH {
        return Err(invalid(format!(
            "core platform `{os}/{arch}` does not match host `{}/{}`",
            std::env::consts::OS,
            std::env::consts::ARCH
        )));
    }
    Ok(())
}

fn validate_family_format(family: &str, format: &str) -> Result<(), io::Error> {
    if !matches!((family, format), ("clash", "yaml") | ("sing-box", "json")) {
        return Err(invalid(format!(
            "unsupported core family/format combination `{family}/{format}`"
        )));
    }
    Ok(())
}

fn validate_file_name(value: &str, field: &str) -> Result<(), io::Error> {
    let path = Path::new(value);
    let mut components = path.components();
    if value.is_empty()
        || path.is_absolute()
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err(invalid(format!(
            "{field} must be a single relative file name: `{value}`"
        )));
    }
    Ok(())
}

fn validate_command(name: &str, command: &CommandArgs) -> Result<(), io::Error> {
    for argument in &command.args {
        let remaining = argument.replace("{config}", "").replace("{workdir}", "");
        if remaining.contains('{') || remaining.contains('}') {
            return Err(invalid(format!(
                "{name} contains an unsupported placeholder in `{argument}`"
            )));
        }
    }
    Ok(())
}

fn dir_name(dir: &Path) -> Result<&str, io::Error> {
    dir.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            invalid(format!(
                "core dir has no valid UTF-8 name: {}",
                dir.display()
            ))
        })
}

fn invalid(message: String) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("invalid core: {message}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const MIHOMO: &str = r#"
schema_version = 1

[core]
name = "mihomo"
family = "clash"
version = "1.19.14"
binary = "mihomo"
os = "linux"
arch = "x86_64"

[runtime]
entrypoint = "config.yaml"
format = "yaml"

[capabilities.config]
mixed_proxy = true
http_proxy = true
socks_proxy = true
api = true
dns = true
tun = true

[commands.start]
args = ["-d", "{workdir}", "-f", "{config}"]

[commands.check]
args = ["-t", "-d", "{workdir}", "-f", "{config}"]

[commands.version]
args = ["-v"]
"#;

    const SING_BOX: &str = r#"
schema_version = 1

[core]
name = "sing-box"
family = "sing-box"
version = "1.13.0"
binary = "sing-box"
os = "linux"
arch = "x86_64"

[runtime]
entrypoint = "config.json"
format = "json"

[capabilities.config]
mixed_proxy = true
http_proxy = false
socks_proxy = false
api = true
dns = true
tun = true

[commands.start]
args = ["run", "-D", "{workdir}", "-c", "{config}"]

[commands.check]
args = ["check", "-D", "{workdir}", "-c", "{config}"]

[commands.version]
args = ["version"]
"#;

    fn write_core(root: &Path, name: &str, binary: &str, body: &str) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).expect("mkdir");
        fs::write(dir.join("core.toml"), body).expect("write core.toml");
        let binary = dir.join(binary);
        fs::write(&binary, "fake core").expect("write binary");
        let mut permissions = fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&binary, permissions).unwrap();
        dir
    }

    fn temp_root(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}", std::process::id()))
    }

    #[test]
    fn loads_mihomo_and_sing_box_command_forms() {
        let root = temp_root("tz-cores-ok");
        fs::create_dir_all(&root).unwrap();
        write_core(&root, "mihomo", "mihomo", MIHOMO);
        write_core(&root, "sing-box", "sing-box", SING_BOX);
        let cores = list_cores(&root).expect("list");
        assert_eq!(cores.len(), 2);
        assert_eq!(cores[0].name, "mihomo");
        assert_eq!(cores[1].name, "sing-box");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_dir_name_mismatch() {
        let root = temp_root("tz-cores-mismatch");
        fs::create_dir_all(&root).unwrap();
        write_core(&root, "mihomo-15", "mihomo", MIHOMO);
        let error = list_cores(&root).expect_err("should reject mismatch");
        assert!(error.to_string().contains("name mismatch"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_unsafe_binary_and_unknown_placeholder() {
        let root = temp_root("tz-cores-invalid");
        fs::create_dir_all(&root).unwrap();
        let unsafe_binary = MIHOMO.replace("binary = \"mihomo\"", "binary = \"../mihomo\"");
        let dir = root.join("mihomo");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("core.toml"), unsafe_binary).unwrap();
        assert!(load_manifest(&dir).is_err());

        let bad_placeholder = MIHOMO.replace("{config}", "{unknown}");
        fs::write(dir.join("core.toml"), bad_placeholder).unwrap();
        let binary = dir.join("mihomo");
        fs::write(&binary, "fake").unwrap();
        let mut permissions = fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(binary, permissions).unwrap();
        assert!(load_manifest(&dir).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_non_executable_binary_and_schema_version() {
        let root = temp_root("tz-cores-permission");
        fs::create_dir_all(&root).unwrap();
        let dir = write_core(&root, "mihomo", "mihomo", MIHOMO);
        let binary = dir.join("mihomo");
        let mut permissions = fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o644);
        fs::set_permissions(&binary, permissions).unwrap();
        assert!(load_manifest(&dir).is_err());

        let mut permissions = fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&binary, permissions).unwrap();
        fs::write(
            dir.join("core.toml"),
            MIHOMO.replace("schema_version = 1", "schema_version = 2"),
        )
        .unwrap();
        assert!(load_manifest(&dir).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_missing_or_mismatched_platform() {
        let root = temp_root("tz-cores-platform");
        fs::create_dir_all(&root).unwrap();
        let dir = root.join("mihomo");
        fs::create_dir_all(&dir).unwrap();

        fs::write(
            dir.join("core.toml"),
            MIHOMO.replace("os = \"linux\"\n", ""),
        )
        .unwrap();
        assert!(load_manifest(&dir).is_err());

        let wrong_os = if std::env::consts::OS == "linux" {
            "windows"
        } else {
            "linux"
        };
        let body = MIHOMO
            .replace("os = \"linux\"", &format!("os = \"{wrong_os}\""))
            .replace(
                "arch = \"x86_64\"",
                &format!("arch = \"{}\"", std::env::consts::ARCH),
            );
        fs::write(dir.join("core.toml"), body).unwrap();
        assert!(load_manifest(&dir).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn renders_placeholders() {
        let root = temp_root("tz-cores-args");
        fs::create_dir_all(&root).unwrap();
        let dir = write_core(&root, "mihomo", "mihomo", MIHOMO);
        let manifest = load_manifest(&dir).expect("load");
        let descriptor = CoreDescriptor {
            name: "mihomo".into(),
            dir: dir.clone(),
            manifest,
        };
        let config = PathBuf::from("/tmp/config.yaml");
        let workdir = PathBuf::from("/tmp/workdir");
        let args =
            descriptor.render_args(&descriptor.manifest.commands.start.args, &config, &workdir);
        assert_eq!(args, vec!["-d", "/tmp/workdir", "-f", "/tmp/config.yaml"]);
        fs::remove_dir_all(&root).unwrap();
    }
}
