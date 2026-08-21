use std::io;

use crate::cli::SettingCommand;
use crate::platform::AppPaths;

pub fn run(command: Option<SettingCommand>, paths: Option<&AppPaths>) -> Result<(), io::Error> {
    let paths = require_paths(paths)?;
    match command {
        None => crate::application::setting::interactive(paths),
        Some(SettingCommand::List) => crate::application::setting::list(paths),
        Some(SettingCommand::Get { key }) => crate::application::setting::get(paths, &key),
        Some(SettingCommand::Set { key, value }) => {
            crate::application::setting::set(paths, &key, value.as_deref())
        }
        Some(SettingCommand::Reset { key }) => {
            crate::application::setting::reset(paths, key.as_deref())
        }
    }
}

fn require_paths(paths: Option<&AppPaths>) -> Result<&AppPaths, io::Error> {
    paths.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "tz 尚未初始化，请先运行 `tz init`。",
        )
    })
}
