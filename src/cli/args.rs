use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Manage local proxy cores, profiles, and runtime state.
#[derive(Debug, Parser)]
#[command(name = "tz", version, about)]
pub struct Cli {
    /// List, search, and interactively select nodes.
    #[arg(short = 'l', long = "list", value_name = "KEYWORD", num_args = 0..=1, default_missing_value = "")]
    pub quick_list: Option<String>,
    #[command(subcommand)]
    pub command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
pub enum CliCommand {
    /// Initialize the filesystem layout and default files.
    Init,
    /// Show the current runtime status.
    #[command(visible_alias = "st")]
    Status,
    /// Start the selected proxy core.
    #[command(visible_alias = "on")]
    Start,
    /// Stop the running proxy core.
    #[command(visible_alias = "off", alias = "end")]
    Stop,
    /// Restart the selected proxy core.
    #[command(visible_alias = "r")]
    Restart,
    /// List or search nodes from the active profile.
    List { keyword: Option<String> },
    /// Open the profile selector for the current core family.
    Select,
    /// Test proxy node latency through the active core.
    Node {
        #[command(subcommand)]
        command: NodeCommand,
    },
    /// Control TUN mode.
    Tun {
        #[command(subcommand)]
        command: ToggleCommand,
    },
    /// Control terminal and desktop proxy integration.
    Proxy {
        #[command(subcommand)]
        command: ProxyCommand,
    },
    /// Inspect and modify persistent settings.
    #[command(visible_alias = "set")]
    Setting {
        #[command(subcommand)]
        command: Option<SettingCommand>,
    },
    /// Manage profiles.
    #[command(visible_alias = "p")]
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    /// Manage proxy cores.
    #[command(visible_alias = "c")]
    Core {
        #[command(subcommand)]
        command: CoreCommand,
    },
    /// Build, validate, or show the effective core configuration.
    #[command(visible_alias = "cfg")]
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Generate shell completion scripts.
    #[command(visible_alias = "comp")]
    Completion {
        #[command(subcommand)]
        command: CompletionCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum CompletionCommand {
    /// Generate a completion script for the selected shell.
    Generate { shell: CompletionShell },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Build,
    Check,
    Show,
}

#[derive(Debug, Subcommand)]
pub enum NodeCommand {
    /// Test matching nodes and sort them by latency.
    Test {
        keyword: Option<String>,
        #[arg(long, default_value = "https://www.gstatic.com/generate_204")]
        url: String,
        #[arg(long, default_value_t = 1800)]
        timeout: u64,
        /// Select the fastest node after testing.
        #[arg(long)]
        select: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ToggleCommand {
    Status,
    On,
    Off,
}

#[derive(Debug, Subcommand)]
pub enum ProxyCommand {
    Status,
    /// Enable terminal and system proxy state.
    On,
    /// Disable terminal and system proxy state.
    Off,
    /// Print environment exports for eval/source.
    Env {
        #[arg(value_enum, default_value = "bash")]
        shell: CompletionShell,
    },
    /// Print environment unset commands for eval/source.
    Noenv {
        #[arg(value_enum, default_value = "bash")]
        shell: CompletionShell,
    },
    /// Print a persistent shell integration function.
    ShellInit {
        shell: CompletionShell,
    },
    /// Control terminal proxy state only.
    Terminal {
        #[command(subcommand)]
        command: ToggleCommand,
    },
    /// Control GNOME system proxy state only.
    System {
        #[command(subcommand)]
        command: ToggleCommand,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

#[derive(Debug, Subcommand)]
pub enum SettingCommand {
    /// List all supported setting keys and current values.
    List,
    /// Print one setting value.
    Get { key: String },
    /// Set one setting value. Missing value is interactive only.
    Set { key: String, value: Option<String> },
    /// Reset one key, or every public key when omitted.
    Reset { key: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ProfileFamily {
    Clash,
    SingBox,
}

impl ProfileFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clash => "clash",
            Self::SingBox => "sing-box",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum ProfileCommand {
    /// Import a remote URL or local file as a managed profile.
    #[command(visible_alias = "a")]
    Add {
        name: String,
        source: String,
        #[arg(long, value_enum)]
        family: ProfileFamily,
    },
    /// List managed profiles.
    #[command(visible_alias = "l")]
    List {
        #[arg(long, value_enum)]
        family: Option<ProfileFamily>,
        /// List profiles from every supported core family.
        #[arg(long, conflicts_with = "family")]
        all: bool,
    },
    /// Show one profile and its origin.
    #[command(visible_alias = "i")]
    Info { name: String },
    /// Select a profile. Missing name is interactive only.
    #[command(visible_alias = "u")]
    Use { name: Option<String> },
    /// Refresh every remote profile.
    #[command(visible_alias = "up")]
    Update,
    /// Remove a managed profile.
    #[command(visible_alias = "rm")]
    Remove { name: String },
}

#[derive(Debug, Subcommand)]
pub enum CoreCommand {
    /// Import a locally prepared core directory.
    #[command(visible_alias = "a")]
    Add { directory: PathBuf },
    /// List registered proxy cores.
    #[command(visible_alias = "l")]
    List,
    /// Show one core, or the current core when omitted.
    #[command(visible_alias = "i")]
    Info { name: Option<String> },
    /// Select a core. Missing name is interactive only.
    #[command(visible_alias = "u")]
    Use { name: Option<String> },
    /// Remove a managed core.
    #[command(visible_alias = "rm")]
    Remove { name: String },
}

#[cfg(test)]
mod tests {
    use super::{
        Cli, CliCommand, NodeCommand, ProfileCommand, ProfileFamily, ProxyCommand, SettingCommand,
        ToggleCommand,
    };
    use clap::Parser;

    #[test]
    fn parses_setting_and_profile_commands() {
        let cli = Cli::try_parse_from(["tz", "setting", "set", "proxy.mode", "global"])
            .expect("setting parses");
        assert!(matches!(
            cli.command,
            Some(CliCommand::Setting {
                command: Some(SettingCommand::Set { .. })
            })
        ));

        let cli = Cli::try_parse_from(["tz", "p", "up"]).expect("short update parses");
        assert!(matches!(
            cli.command,
            Some(CliCommand::Profile {
                command: ProfileCommand::Update
            })
        ));

        let cli = Cli::try_parse_from(["tz", "select"]).expect("selector parses");
        assert!(matches!(cli.command, Some(CliCommand::Select)));

        let cli = Cli::try_parse_from(["tz", "tun", "on"]).expect("tun parses");
        assert!(matches!(
            cli.command,
            Some(CliCommand::Tun {
                command: ToggleCommand::On
            })
        ));

        let cli =
            Cli::try_parse_from(["tz", "proxy", "terminal", "off"]).expect("terminal proxy parses");
        assert!(matches!(
            cli.command,
            Some(CliCommand::Proxy {
                command: ProxyCommand::Terminal {
                    command: ToggleCommand::Off
                }
            })
        ));

        let cli = Cli::try_parse_from(["tz", "node", "test", "hk", "--select"])
            .expect("node test parses");
        assert!(matches!(
            cli.command,
            Some(CliCommand::Node {
                command: NodeCommand::Test { select: true, .. }
            })
        ));

        let cli = Cli::try_parse_from([
            "tz",
            "profile",
            "add",
            "home",
            "/tmp/home.yaml",
            "--family",
            "clash",
        ])
        .expect("profile parses");
        assert!(matches!(
            cli.command,
            Some(CliCommand::Profile {
                command: ProfileCommand::Add {
                    family: ProfileFamily::Clash,
                    ..
                }
            })
        ));

        let cli = Cli::try_parse_from(["tz", "on"]).expect("start alias parses");
        assert!(matches!(cli.command, Some(CliCommand::Start)));

        for alias in ["off", "end"] {
            let cli = Cli::try_parse_from(["tz", alias]).expect("stop alias parses");
            assert!(matches!(cli.command, Some(CliCommand::Stop)));
        }

        let cli = Cli::try_parse_from(["tz", "-l", "hong"]).expect("quick list parses");
        assert_eq!(cli.quick_list.as_deref(), Some("hong"));

        let cli = Cli::try_parse_from(["tz", "profile", "list", "--all"])
            .expect("profile list all parses");
        assert!(matches!(
            cli.command,
            Some(CliCommand::Profile {
                command: ProfileCommand::List { all: true, .. }
            })
        ));
    }
}
