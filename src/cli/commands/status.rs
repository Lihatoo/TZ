use std::io;

use crate::platform::AppPaths;

pub fn run(paths: Option<&AppPaths>) -> Result<(), io::Error> {
    let Some(paths) = paths else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "tz 尚未初始化，请先运行 `tz init`。",
        ));
    };
    crate::application::status(paths)
}
