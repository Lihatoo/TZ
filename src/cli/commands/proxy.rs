use std::io;

use crate::{
    cli::{CompletionShell, ProxyCommand, ToggleCommand},
    platform::AppPaths,
};

pub fn run(command: ProxyCommand, paths: &AppPaths) -> Result<(), io::Error> {
    match command {
        ProxyCommand::Status => crate::application::proxy::status(paths),
        ProxyCommand::On => crate::application::proxy::both(paths, true),
        ProxyCommand::Off => crate::application::proxy::both(paths, false),
        ProxyCommand::Env { shell } => crate::application::proxy::env(paths, shell_name(shell)),
        ProxyCommand::Noenv { shell } => crate::application::proxy::noenv(shell_name(shell)),
        ProxyCommand::ShellInit { shell } => {
            crate::application::proxy::shell_init(paths, shell_name(shell))
        }
        ProxyCommand::Terminal { command } => match command {
            ToggleCommand::Status => crate::application::proxy::status(paths),
            ToggleCommand::On => crate::application::proxy::terminal(paths, true),
            ToggleCommand::Off => crate::application::proxy::terminal(paths, false),
        },
        ProxyCommand::System { command } => match command {
            ToggleCommand::Status => crate::application::proxy::status(paths),
            ToggleCommand::On => crate::application::proxy::system(paths, true),
            ToggleCommand::Off => crate::application::proxy::system(paths, false),
        },
    }
}

fn shell_name(shell: CompletionShell) -> &'static str {
    match shell {
        CompletionShell::Bash => "bash",
        CompletionShell::Zsh => "zsh",
        CompletionShell::Fish => "fish",
    }
}
