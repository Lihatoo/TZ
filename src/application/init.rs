use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use crate::domain::{ActiveConfig, ProfilesIndex, RuntimeConfig, Settings};
use crate::platform::{AppPaths, LayoutFile, load_paths_file, paths_file, save_paths_file};

const PATHS_FILE_ENV: &str = "TZ_PATHS_TOML";
const BASHRC_FILE: &str = ".bashrc";

pub fn initialize() -> Result<(), Box<dyn std::error::Error>> {
    let paths_file = paths_file()?;
    if paths_file.is_file() {
        println!("已找到路径配置：{}", paths_file.display());
        match load_paths_file(&paths_file) {
            Ok(current) => {
                println!("当前路径：");
                println!("config: {}", current.config_dir.display());
                println!("data:   {}", current.data_dir.display());
                println!("state:  {}", current.state_dir.display());
                println!("cache:  {}", current.cache_dir.display());
            }
            Err(error) => return Err(error.into()),
        }
        let choice = prompt("是否继续重新初始化？[y/N]: ", "")?;
        if !matches!(choice.to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("已退出，未修改路径配置。");
            return Ok(());
        }
    }

    let layout = choose_layout()?;
    let paths = AppPaths::from_layout(layout)?;
    paths.initialize_files()?;
    seed_default_configs(&paths)?;
    save_paths_file(&paths_file, &paths)?;

    println!("初始化完成。");
    println!("paths:  {}", paths_file.display());
    println!("config: {}", paths.config_dir.display());
    println!("data:   {}", paths.data_dir.display());
    println!("state:  {}", paths.state_dir.display());
    println!("cache:  {}", paths.cache_dir.display());
    remind_paths_environment(&paths_file)?;
    Ok(())
}

/// 用 domain 默认值幂等写出 settings.toml / runtime.toml / active.toml / profiles.toml。
/// 已存在的文件一律跳过，不覆盖用户改动。
fn seed_default_configs(paths: &AppPaths) -> Result<(), io::Error> {
    if !paths.settings_file().is_file() {
        Settings::default().save(&paths.settings_file())?;
    }
    if !paths.runtime_file().is_file() {
        RuntimeConfig::default().save(&paths.runtime_file())?;
    }
    if !paths.active_file().is_file() {
        ActiveConfig::default().save(&paths.active_file())?;
    }
    if !paths.profiles_file().is_file() {
        ProfilesIndex::default().save(&paths.profiles_file())?;
    }
    Ok(())
}

fn choose_layout() -> Result<LayoutFile, io::Error> {
    let templates = templates()?;
    println!("选择路径模板：");
    for (index, (name, layout)) in templates.iter().enumerate() {
        println!("  {}) {name}", index + 1);
        print_layout(layout);
    }

    let choice = prompt("请选择 [1/2/3]（默认 1）: ", "1")?;
    let index = choice
        .parse::<usize>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "请输入 1、2 或 3"))?;
    let Some((_, template)) = templates.get(index.saturating_sub(1)) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "请输入 1、2 或 3",
        ));
    };

    let config_dir = prompt_path("config_dir", &template.config_dir)?;
    let data_dir = prompt_path("data_dir", &template.data_dir)?;
    let state_dir = prompt_path("state_dir", &template.state_dir)?;
    let cache_dir = prompt_path("cache_dir", &template.cache_dir)?;

    Ok(LayoutFile {
        config_dir,
        data_dir,
        state_dir,
        cache_dir,
    })
}

fn templates() -> Result<Vec<(&'static str, LayoutFile)>, io::Error> {
    let development_root = env::current_dir()
        .map_err(io::Error::other)?
        .join("target/tz-dev");
    Ok(vec![
        (
            "默认 XDG",
            LayoutFile {
                config_dir: "~/.config/tz".into(),
                data_dir: "~/.local/share/tz".into(),
                state_dir: "~/.local/state/tz".into(),
                cache_dir: "~/.cache/tz".into(),
            },
        ),
        (
            "默认 Unified",
            LayoutFile {
                config_dir: "~/.tz/config".into(),
                data_dir: "~/.tz/data".into(),
                state_dir: "~/.tz/state".into(),
                cache_dir: "~/.tz/cache".into(),
            },
        ),
        (
            "开发测试 target/tz-dev",
            LayoutFile {
                config_dir: development_root.join("config").display().to_string(),
                data_dir: development_root.join("data").display().to_string(),
                state_dir: development_root.join("state").display().to_string(),
                cache_dir: development_root.join("cache").display().to_string(),
            },
        ),
    ])
}

fn print_layout(layout: &LayoutFile) {
    println!("     config: {}", layout.config_dir);
    println!("     data:   {}", layout.data_dir);
    println!("     state:  {}", layout.state_dir);
    println!("     cache:  {}", layout.cache_dir);
}

fn prompt_path(name: &str, default: &str) -> Result<String, io::Error> {
    let value = prompt(&format!("{name} [{default}]: "), "")?;
    if value.is_empty() {
        return Ok(default.to_owned());
    }
    let path = PathBuf::from(&value);
    if !path.is_absolute() && !value.starts_with("~/") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} 必须是绝对路径或以 ~/ 开头"),
        ));
    }
    Ok(value)
}

fn remind_paths_environment(file: &Path) -> Result<(), io::Error> {
    let home = home_dir()?;
    let default_file = home.join(".config/tz/paths.toml");
    let export = format!("export {PATHS_FILE_ENV}={}", shell_quote(file));
    println!("如需显式指定路径配置，可执行：{export}");

    if file == default_file {
        println!("当前 paths.toml 位于默认位置，不设置环境变量也可以使用。");
        return Ok(());
    }

    let choice = prompt("是否将路径配置追加到 ~/.bashrc？[y/N]: ", "")?;
    if !matches!(choice.to_ascii_lowercase().as_str(), "y" | "yes") {
        println!("未修改 ~/.bashrc；新 shell 需要上述 export 才能找到此 paths.toml。");
        return Ok(());
    }

    let bashrc = home.join(BASHRC_FILE);
    let existing = fs::read_to_string(&bashrc).unwrap_or_default();
    if existing
        .lines()
        .any(|line| line.starts_with("export TZ_PATHS_TOML="))
        && !existing.lines().any(|line| line == export)
    {
        println!(
            "~/.bashrc 已存在不同的 TZ_PATHS_TOML，未覆盖。请手动检查：{}",
            bashrc.display()
        );
        return Ok(());
    }
    append_unique_line(&bashrc, &export)?;
    println!(
        "已将环境变量追加到 {}；重新打开 shell 或执行 source ~/.bashrc 后生效。",
        bashrc.display()
    );
    Ok(())
}

fn append_unique_line(path: &Path, line: &str) -> Result<(), io::Error> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    if existing.lines().any(|current| current == line) {
        return Ok(());
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        writeln!(file)?;
    }
    writeln!(file, "{line}")?;
    Ok(())
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn prompt(message: &str, default: &str) -> Result<String, io::Error> {
    print!("{message}");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let value = input.trim();
    Ok(if value.is_empty() {
        default.to_owned()
    } else {
        value.to_owned()
    })
}

fn home_dir() -> Result<PathBuf, io::Error> {
    env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME 未设置或为空"))
}

#[cfg(test)]
mod tests {
    use super::append_unique_line;

    #[test]
    fn paths_export_is_appended_once() {
        let path = std::env::temp_dir().join(format!("tz-bashrc-test-{}", std::process::id()));
        let line = "export TZ_PATHS_TOML='/tmp/paths.toml'";
        append_unique_line(&path, line).expect("append should work");
        append_unique_line(&path, line).expect("second append should work");
        let content = std::fs::read_to_string(&path).expect("read fixture");
        assert_eq!(content.matches(line).count(), 1);
        std::fs::remove_file(path).expect("remove fixture");
    }
}
