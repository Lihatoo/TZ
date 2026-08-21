use std::{fs, io};

use crate::{cli::ConfigCommand, platform::AppPaths};

pub fn run(command: ConfigCommand, paths: Option<&AppPaths>) -> Result<(), io::Error> {
    let paths = paths.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "tz 尚未初始化，请先运行 `tz init`。",
        )
    })?;
    match command {
        ConfigCommand::Build => {
            let built = crate::application::build_config(paths)?;
            println!(
                "已生成 core={} profile={} config={}",
                built.core.name,
                built.profile_name,
                built.config_path.display()
            );
        }
        ConfigCommand::Check => {
            let built = crate::application::check_config(paths)?;
            println!(
                "配置有效 core={} profile={}",
                built.core.name, built.profile_name
            );
        }
        ConfigCommand::Show => {
            let built = crate::application::build_config(paths)?;
            print!("{}", fs::read_to_string(built.config_path)?);
        }
    }
    Ok(())
}
