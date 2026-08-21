use std::{io, io::IsTerminal};

use crate::application::{AddProfile, ProfileError, ProfileService};
use crate::cli::ProfileCommand;
use crate::domain::{ActiveConfig, ProfilesIndex};
use crate::platform::{AppPaths, SecureDownloader};

pub fn run(
    command: ProfileCommand,
    paths: Option<&AppPaths>,
) -> Result<(), Box<dyn std::error::Error>> {
    let paths = require_paths(paths)?;
    let downloader = SecureDownloader::default();
    let service = ProfileService::new(paths, &downloader);
    match command {
        ProfileCommand::Add {
            name,
            source,
            family,
        } => {
            let downloader = SecureDownloader::for_family(family.as_str());
            let service = ProfileService::new(paths, &downloader);
            let entry = service.add(AddProfile {
                name: &name,
                family: family.as_str(),
                source: &source,
            })?;
            println!(
                "已添加 profile {} family={} source={}",
                entry.name, entry.family, entry.origin.kind
            );
            println!("使用 `tz profile use {}` 选择它。", entry.name);
        }
        ProfileCommand::List { family, all } => {
            let current_family;
            let family = if all {
                None
            } else if let Some(family) = family {
                Some(family.as_str())
            } else {
                current_family = current_core_family(paths)?;
                Some(current_family.as_str())
            };
            let profiles = service.list(family)?;
            if profiles.is_empty() {
                println!("暂无 profile");
            } else {
                for (index, profile) in profiles.iter().enumerate() {
                    let marker = if profile.current { "*" } else { " " };
                    if io::stdin().is_terminal() {
                        println!(
                            "{marker} {}) {} family={}",
                            index + 1,
                            profile.name,
                            profile.family
                        );
                    } else {
                        println!("{marker} {} family={}", profile.name, profile.family);
                    }
                }
                if io::stdin().is_terminal() {
                    let selected = prompt_index("选择 profile（0 保持当前）: ", profiles.len())?;
                    if let Some(selected) = selected {
                        let entry = service.use_profile(&profiles[selected].name)?;
                        println!("当前 {} profile: {}", entry.family, entry.name);
                    }
                }
            }
        }
        ProfileCommand::Info { name } => {
            let entry = service.info(&name)?;
            let index = ProfilesIndex::load(&paths.profiles_file())?;
            println!("name        : {}", entry.name);
            println!("family      : {}", entry.family);
            println!("format      : {}", entry.format);
            println!("source      : {}", entry.source_file);
            println!("origin      : {}", entry.origin.kind);
            if entry.origin.kind == "remote" {
                println!("url         : <redacted>");
                println!(
                    "download_via: {}",
                    if entry.origin.download_via.is_empty() {
                        "unknown"
                    } else {
                        &entry.origin.download_via
                    }
                );
            } else {
                println!("original    : {}", entry.origin.original_path);
            }
            println!(
                "current     : {}",
                yes_no(index.current.get(&entry.family) == Some(&entry.name))
            );
            if !entry.update.updated_at.is_empty() {
                println!("updated_at  : {}", entry.update.updated_at);
            }
        }
        ProfileCommand::Use { name } => {
            let name = select_name(paths, name.as_deref())?;
            let entry = service.use_profile(&name)?;
            println!("当前 {} profile: {}", entry.family, entry.name);
        }
        ProfileCommand::Update => {
            crate::platform::ensure_not_running(&paths.core_pid_file())?;
            let index = ProfilesIndex::load(&paths.profiles_file())?;
            let profiles: Vec<_> = index
                .profiles
                .iter()
                .filter(|profile| profile.origin.kind == "remote")
                .map(|profile| (profile.name.clone(), profile.family.clone()))
                .collect();
            if profiles.is_empty() {
                println!("没有可更新的远程 profile");
                return Ok(());
            }
            let mut failures = Vec::new();
            for (name, family) in profiles {
                let downloader = SecureDownloader::for_family(&family);
                let service = ProfileService::new(paths, &downloader);
                match service.update(&name) {
                    Ok(entry) => {
                        println!("已更新 {} via={}", entry.name, entry.origin.download_via)
                    }
                    Err(error) => {
                        eprintln!("更新失败 {name}: {error}");
                        failures.push(name);
                    }
                }
            }
            if !failures.is_empty() {
                return Err(ProfileError::InvalidInput(format!(
                    "{} 个 profile 更新失败: {}",
                    failures.len(),
                    failures.join(", ")
                ))
                .into());
            }
        }
        ProfileCommand::Remove { name } => {
            let removed = service.remove(&name)?;
            println!("已删除 profile {}", removed.name);
        }
    }
    Ok(())
}

fn select_name(paths: &AppPaths, name: Option<&str>) -> Result<String, ProfileError> {
    match name {
        Some(name) => Ok(name.to_owned()),
        None if io::stdin().is_terminal() => choose_profile(paths),
        None => Err(ProfileError::InvalidInput(
            "非交互调用必须提供 profile name".into(),
        )),
    }
}

fn choose_profile(paths: &AppPaths) -> Result<String, ProfileError> {
    use std::io::Write;
    let downloader = SecureDownloader::default();
    let service = ProfileService::new(paths, &downloader);
    let family = current_core_family(paths)?;
    let profiles = service.list(Some(&family))?;
    if profiles.is_empty() {
        return Err(ProfileError::NotFound("any profile".into()));
    }
    for (index, profile) in profiles.iter().enumerate() {
        println!("{}) {} ({})", index + 1, profile.name, profile.family);
    }
    print!("选择 profile（0 取消）: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let index = input
        .trim()
        .parse::<usize>()
        .map_err(|_| ProfileError::InvalidInput("请输入列表中的数字".into()))?;
    if index == 0 {
        return Err(ProfileError::InvalidInput("已取消".into()));
    }
    profiles
        .get(index.saturating_sub(1))
        .map(|profile| profile.name.clone())
        .ok_or_else(|| ProfileError::InvalidInput("选择超出范围".into()))
}

fn current_core_family(paths: &AppPaths) -> Result<String, ProfileError> {
    let active = ActiveConfig::load(&paths.active_file())?;
    if active.current.core.is_empty() {
        return Err(ProfileError::InvalidInput(
            "未选择 core，默认 profile list 需要先选择 core；如需查看全部请使用 --all".into(),
        ));
    }
    let manifest = crate::domain::load_manifest(&paths.cores_dir().join(active.current.core))?;
    Ok(manifest.core.family)
}

fn prompt_index(message: &str, count: usize) -> Result<Option<usize>, ProfileError> {
    use std::io::Write;
    print!("{message}");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let value = input
        .trim()
        .parse::<usize>()
        .map_err(|_| ProfileError::InvalidInput("请输入列表中的数字".into()))?;
    if value == 0 {
        return Ok(None);
    }
    if value > count {
        return Err(ProfileError::InvalidInput("选择超出范围".into()));
    }
    Ok(Some(value - 1))
}

fn require_paths(paths: Option<&AppPaths>) -> Result<&AppPaths, io::Error> {
    paths.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "tz 尚未初始化，请先运行 `tz init`。",
        )
    })
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
