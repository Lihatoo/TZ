use std::io;

use crate::platform::AppPaths;

pub fn start(paths: &AppPaths) -> Result<(), io::Error> {
    crate::application::start(paths)?;
    crate::application::status(paths)
}

pub fn stop(paths: &AppPaths) -> Result<(), io::Error> {
    crate::application::stop(paths)
}

pub fn restart(paths: &AppPaths) -> Result<(), io::Error> {
    crate::application::restart(paths)?;
    crate::application::status(paths)
}

pub fn list(paths: &AppPaths, keyword: Option<&str>) -> Result<(), io::Error> {
    crate::application::list(paths, keyword)
}
