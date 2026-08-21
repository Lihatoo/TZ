use std::io;

use crate::{application::NodeTestOptions, cli::NodeCommand, platform::AppPaths};

pub fn run(command: NodeCommand, paths: &AppPaths) -> Result<(), io::Error> {
    match command {
        NodeCommand::Test {
            keyword,
            url,
            timeout,
            select,
        } => crate::application::test_nodes(
            paths,
            NodeTestOptions {
                keyword: keyword.as_deref(),
                url: &url,
                timeout_ms: timeout,
                select,
            },
        ),
    }
}
