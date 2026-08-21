mod args;
mod commands;
use std::error::Error;

use crate::platform::AppPaths;

pub use args::{
    Cli, CliCommand, CompletionCommand, CompletionShell, ConfigCommand, CoreCommand, NodeCommand,
    ProfileCommand, ProfileFamily, ProxyCommand, SettingCommand, ToggleCommand,
};

pub fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    let command = match (cli.quick_list, cli.command) {
        (Some(keyword), None) => CliCommand::List {
            keyword: (!keyword.is_empty()).then_some(keyword),
        },
        (Some(_), Some(_)) => {
            return Err("-l/--list cannot be combined with another command".into());
        }
        (None, Some(command)) => command,
        (None, None) => CliCommand::Status,
    };
    if let CliCommand::Completion { command } = command {
        commands::completion::run(command)?;
        return Ok(());
    }

    let paths = if matches!(command, CliCommand::Init) {
        None
    } else {
        AppPaths::from_env_or_none()?
    };

    match command {
        CliCommand::Init => commands::init::run()?,
        CliCommand::Status => commands::status::run(paths.as_ref())?,
        CliCommand::Start => commands::service::start(require_paths(paths.as_ref())?)?,
        CliCommand::Stop => commands::service::stop(require_paths(paths.as_ref())?)?,
        CliCommand::Restart => commands::service::restart(require_paths(paths.as_ref())?)?,
        CliCommand::List { keyword } => {
            commands::service::list(require_paths(paths.as_ref())?, keyword.as_deref())?
        }
        CliCommand::Select => commands::profile::run(
            ProfileCommand::List {
                family: None,
                all: false,
            },
            paths.as_ref(),
        )?,
        CliCommand::Node { command } => {
            commands::node::run(command, require_paths(paths.as_ref())?)?
        }
        CliCommand::Tun { command } => commands::tun::run(command, require_paths(paths.as_ref())?)?,
        CliCommand::Proxy { command } => {
            commands::proxy::run(command, require_paths(paths.as_ref())?)?
        }
        CliCommand::Setting { command } => commands::setting::run(command, paths.as_ref())?,
        CliCommand::Profile { command } => commands::profile::run(command, paths.as_ref())?,
        CliCommand::Core { command } => commands::core::run(command, paths.as_ref())?,
        CliCommand::Config { command } => commands::config::run(command, paths.as_ref())?,
        CliCommand::Completion { .. } => unreachable!("completion handled before path resolution"),
    }

    Ok(())
}

fn require_paths(paths: Option<&AppPaths>) -> Result<&AppPaths, std::io::Error> {
    paths.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "tz 尚未初始化，请先运行 `tz init`。",
        )
    })
}
