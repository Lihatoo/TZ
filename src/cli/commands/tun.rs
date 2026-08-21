use std::io;

use crate::{cli::ToggleCommand, platform::AppPaths};

pub fn run(command: ToggleCommand, paths: &AppPaths) -> Result<(), io::Error> {
    match command {
        ToggleCommand::Status => crate::application::tun::status(paths),
        ToggleCommand::On => crate::application::tun::set(paths, true),
        ToggleCommand::Off => crate::application::tun::set(paths, false),
    }
}
