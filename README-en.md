[中文](./README.md) | [English](./README-en.md)

# TZ

TZ is a terminal proxy manager for unified management of Mihomo, the sing-box core, subscription profiles, node latency tests, TUN, and terminal/system proxies. The current version is `v0.1.0` and supports Linux x86_64.

## Installation

### Cargo

The Rust 2024 edition toolchain is required. Install the release version from the repository directory:

```bash
git clone https://github.com/Lihatoo/TZ.git
cd TZ
cargo install --path .
```

The default installation path is `~/.cargo/bin/tz`. Make sure `~/.cargo/bin` is included in your `PATH`.

### Release binary

You can also download `tz` from Releases, make it executable, and place it in a directory on your `PATH`, such as `~/.local/bin`.

## Quick Start

Initialize the directories before using TZ for the first time:

```bash
tz init
```

The default path configuration is located at `~/.config/tz/paths.toml`. To use a custom location, set `TZ_PATHS_TOML` as prompted during initialization.

Import the core directories prepared in the repository:

```bash
tz core add ./cores/mihomo
tz core add ./cores/sing-box
tz core list
tz core use mihomo
```

The argument to `tz core add` must be a complete directory containing `core.toml` and the binary; it cannot be just the binary file. The two built-in cores correspond as follows:

| core | profile family | profile format |
| --- | --- | --- |
| `mihomo` | `clash` | YAML |
| `sing-box` | `sing-box` | JSON |

Add and select profiles:

```bash
tz profile add nano-clash '<subscription URL or local file>' --family clash
tz profile add nano-sb '<subscription URL or local file>' --family sing-box
tz profile list
```

By default, `tz profile list` only lists families supported by the current core. In an interactive terminal, enter a number to select a profile directly; `*` marks the current profile. Use `tz profile list --all` to view all families. Profile names must be unique across all families. Adding a `-clash` or `-sb` suffix is recommended for easier identification.

Start TZ and view its status:

```bash
tz on
tz
```

`tz on` uses the last valid profile selected for the current core. To switch the core or profile, run `tz off` first. Switching is rejected while TZ is running to prevent the recorded state from diverging from the actual process.

## Nodes and Proxies

```bash
tz -l                 # Test all nodes, sort by latency, and select interactively
tz -l hk              # Search, test, and select nodes whose names contain hk
tz node test --select # Test nodes and automatically select the fastest one
```

Terminal proxies must be `eval`-ed in the current shell to take effect:

```bash
eval "$(tz proxy env bash)"
eval "$(tz proxy noenv bash)"
```

For Zsh or Fish, replace the trailing `bash` with the corresponding shell. You can also install a shell hook so that `tz proxy terminal on|off` can modify the current shell:

```bash
eval "$(tz proxy shell-init bash)"
```

After the core starts, you can control the GNOME system proxy or control both the terminal and system proxies:

```bash
tz proxy system on
tz proxy system off
tz proxy on
tz proxy off
```

TUN is independent of the proxy switches above. Its configuration is checked after changes, and the service restarts automatically while it is running:

```bash
tz tun status
tz tun on
tz tun off
```

Enabling TUN requires `/dev/net/tun` to exist on the system. Grant the current core binary `CAP_NET_ADMIN`/`CAP_NET_RAW` as instructed by any command errors.

## Profile Download and Updates

For remote profiles, TZ attempts downloads both through an existing TZ proxy and via a direct connection. As long as either route succeeds, the successful route is recorded as `download_via`. Use `tz profile info <name>` to view this information; URLs are stored only in the local profile index and are hidden from command output.

Download requests use the corresponding client's User-Agent for each family. TZ only validates and manages the original formats; it does not convert Clash YAML to sing-box JSON or vice versa.

```bash
tz profile update       # Update all remote profiles
tz profile info nano-sb
tz profile remove nano-sb
```

If neither a direct connection nor the current TZ proxy can download a profile, start an available TZ profile first, or temporarily enable another proxy and retry.

`Country.mmdb` and `GeoSite.dat` in the Mihomo core directory are GEOIP/GEOSITE rule databases, not plugins that each user needs to install separately. When a profile uses the corresponding rules, TZ copies these files into the runtime directory to prevent Mihomo from attempting a temporary download from GitHub at startup.

## Shell Completion

Enable completion temporarily in the current shell:

```bash
# Bash
eval "$(tz completion generate bash)"

# Zsh
eval "$(tz completion generate zsh)"

# Fish
tz completion generate fish | source
```

To enable completion permanently, add the corresponding command to your shell's startup file.

## Full Commands

```text
tz status|start|stop|restart
tz list [keyword]
tz node test [keyword] [--url <url>] [--timeout <ms>] [--select]
tz tun status|on|off
tz proxy status|on|off
tz proxy terminal|system status|on|off
tz proxy env|noenv [bash|zsh|fish]
tz proxy shell-init bash|zsh|fish
tz setting [list|get|set|reset]
tz profile add|list|info|use|update|remove
tz core add|list|info|use|remove
tz config build|check|show
tz completion generate bash|zsh|fish
```

## Short Commands

```bash
tz                 # Show status and test the current node
tz on              # Start with the last valid profile and show status
tz off             # Stop
tz -l [keyword]     # Test nodes, sort by latency, search, and select
tz select           # List and select a profile from the current family
```

## Shortcuts

Shortcuts are abbreviations for the full commands:

```text
tz st                         -> tz status
tz r                          -> tz restart
tz end                        -> tz stop
tz set                        -> tz setting
tz p                          -> tz profile
tz c                          -> tz core
tz cfg                        -> tz config
tz comp                       -> tz completion
tz p a|l|i|u|up|rm            -> add|list|info|use|update|remove
tz c a|l|i|u|rm               -> add|list|info|use|remove
```

Run `tz --help` or `tz <command> --help` for detailed parameter information.
