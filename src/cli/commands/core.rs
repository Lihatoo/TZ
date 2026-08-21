use std::{io, io::IsTerminal};

use crate::cli::CoreCommand;
use crate::domain::list_cores;
use crate::platform::AppPaths;

pub fn run(command: CoreCommand, paths: Option<&AppPaths>) -> Result<(), io::Error> {
    let paths = require_paths(paths)?;
    match command {
        CoreCommand::Add { directory } => {
            let added = crate::application::add_core(paths, &directory)?;
            println!(
                "已导入 core {} family={} version={}",
                added.descriptor.name,
                added.descriptor.manifest.core.family,
                added.descriptor.manifest.core.version
            );
            if let Some(output) = added.version_output.filter(|output| !output.is_empty()) {
                println!("version output: {output}");
            }
            Ok(())
        }
        CoreCommand::List => list(paths),
        CoreCommand::Info { name } => info(paths, name.as_deref()),
        CoreCommand::Use { name } => select(paths, name.as_deref()),
        CoreCommand::Remove { name } => {
            crate::application::remove_core(paths, &name)?;
            println!("已删除 core {name}");
            Ok(())
        }
    }
}

fn list(paths: &AppPaths) -> Result<(), io::Error> {
    let active = crate::domain::ActiveConfig::load(&paths.active_file())?;
    let cores = list_cores(&paths.cores_dir())?;
    if cores.is_empty() {
        println!(
            "暂无已注册 core，请把 core 目录放到 {}",
            paths.cores_dir().display()
        );
        return Ok(());
    }
    for (index, core) in cores.iter().enumerate() {
        let marker = if active.current.core == core.name {
            "*"
        } else {
            " "
        };
        let manifest = &core.manifest;
        if io::stdin().is_terminal() {
            println!(
                "{marker} {}) {} version={} family={}",
                index + 1,
                core.name,
                manifest.core.version,
                manifest.core.family
            );
        } else {
            println!(
                "{marker} {} version={} family={}",
                core.name, manifest.core.version, manifest.core.family
            );
        }
    }
    if io::stdin().is_terminal() {
        let value = prompt("选择 core（0 保持当前）: ")?;
        let selected = value
            .parse::<usize>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "请输入列表中的数字"))?;
        if selected > cores.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "选择超出范围"));
        }
        if selected > 0 {
            let name = &cores[selected - 1].name;
            crate::application::use_core(paths, name)?;
            println!("当前 core: {name}");
        }
    }
    Ok(())
}

fn info(paths: &AppPaths, name: Option<&str>) -> Result<(), io::Error> {
    let info = crate::application::core_info(paths, name)?;
    let core = &info.descriptor;
    let manifest = &core.manifest;
    println!("name       : {}", core.name);
    println!("family     : {}", manifest.core.family);
    println!("version    : {}", manifest.core.version);
    println!("platform   : {}/{}", manifest.core.os, manifest.core.arch);
    println!("directory  : {}", core.dir.display());
    println!("binary     : {}", core.binary_path().display());
    println!(
        "entrypoint : {} ({})",
        manifest.runtime.entrypoint, manifest.runtime.format
    );
    println!(
        "commands   : start=yes check={} version={} reload={}",
        yes_no(manifest.commands.check.is_some()),
        yes_no(manifest.commands.version.is_some()),
        yes_no(manifest.commands.reload.is_some())
    );
    if let Some(output) = info.version_output.filter(|output| !output.is_empty()) {
        println!("actual     : {output}");
        if !output.contains(&manifest.core.version) {
            println!("warning    : version output does not contain manifest version");
        }
    }
    Ok(())
}

fn select(paths: &AppPaths, name: Option<&str>) -> Result<(), io::Error> {
    let selected = match name {
        Some(name) => name.to_owned(),
        None if io::stdin().is_terminal() => choose_core(paths)?,
        None => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "非交互调用必须提供 core name",
            ));
        }
    };
    crate::application::use_core(paths, &selected)?;
    println!("当前 core: {selected}");
    Ok(())
}

fn choose_core(paths: &AppPaths) -> Result<String, io::Error> {
    let cores = list_cores(&paths.cores_dir())?;
    if cores.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "没有可选择的 core"));
    }
    for (index, core) in cores.iter().enumerate() {
        println!(
            "{}) {} ({})",
            index + 1,
            core.name,
            core.manifest.core.version
        );
    }
    let value = prompt("选择 core（0 取消）: ")?;
    let index = value
        .parse::<usize>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "请输入列表中的数字"))?;
    if index == 0 {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "已取消"));
    }
    cores
        .get(index.saturating_sub(1))
        .map(|core| core.name.clone())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "选择超出范围"))
}

fn prompt(message: &str) -> Result<String, io::Error> {
    use std::io::Write;
    print!("{message}");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_owned())
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
