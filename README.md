[中文](./README.md) | [English](./README-en.md)

# TZ

TZ 是一个终端代理管理器，用于统一管理 Mihomo、sing-box core、订阅 profile、节点测速、TUN 以及终端/系统代理。当前版本为 `v0.1.0`，支持 Linux x86_64。

## 安装

### Cargo

需要 Rust 2024 edition 对应的工具链。在仓库目录安装 release 版本：

```bash
git clone https://github.com/Lihatoo/TZ.git
cd TZ
cargo install --path .
```

默认安装到 `~/.cargo/bin/tz`。请确认 `~/.cargo/bin` 已加入 `PATH`。

### Release 二进制

也可以从 Releases 下载 `tz`，赋予执行权限后放入 `PATH` 中的目录，例如 `~/.local/bin`。

## 快速开始

首次使用先初始化目录：

```bash
tz init
```

默认路径配置位于 `~/.config/tz/paths.toml`。选择自定义位置时，按初始化提示设置 `TZ_PATHS_TOML`。

导入仓库中准备好的 core 目录：

```bash
tz core add ./cores/mihomo
tz core add ./cores/sing-box
tz core list
tz core use mihomo
```

`tz core add` 的参数必须是包含 `core.toml` 和二进制的完整目录，不能只传二进制文件。两个内置 core 的对应关系如下：

| core | profile family | profile 格式 |
| --- | --- | --- |
| `mihomo` | `clash` | YAML |
| `sing-box` | `sing-box` | JSON |

cores均来自官方的mihomo 较新、sing-box 13 内核，更新可以尝试直接下载内核替换即可

添加并选择 profile：

```bash
tz profile add nano-clash '<订阅 URL 或本地文件>' --family clash
tz profile add nano-sb '<订阅 URL 或本地文件>' --family sing-box
tz profile list
```

`tz profile list` 默认只列出当前 core 支持的 family，在交互式终端中可直接输入序号选择；`*` 表示当前 profile。使用 `tz profile list --all` 查看全部 family。profile 名称在所有 family 中必须唯一，建议加入 `-clash` 或 `-sb` 后缀，便于识别。

启动并查看状态：

```bash
tz on
tz
```

`tz on` 使用当前 core 上次选择的有效 profile。core 或 profile 需要切换时先执行 `tz off`，运行期间会拒绝切换，以免状态与实际进程不一致。

## 节点与代理

```bash
tz -l                 # 测速全部节点，按延迟排序，并可交互选择
tz -l hk              # 搜索、测速并选择名称包含 hk 的节点
tz node test --select # 测速并自动选择最快节点
```

终端代理必须在当前 shell 中 `eval` 才能生效：

```bash
eval "$(tz proxy env bash)"
eval "$(tz proxy noenv bash)"
```

Zsh 或 Fish 将末尾的 `bash` 换成对应 shell。也可以安装 shell hook，使 `tz proxy terminal on|off` 能修改当前 shell：

```bash
eval "$(tz proxy shell-init bash)"
```

core 启动后，可以控制 GNOME 系统代理或同时控制终端与系统代理：

```bash
tz proxy system on
tz proxy system off
tz proxy on
tz proxy off
```

TUN 独立于上述代理开关；修改后会校验配置，服务运行时自动重启：

```bash
tz tun status
tz tun on
tz tun off
```

启用 TUN 需要系统存在 `/dev/net/tun`，并按命令报错提示为当前 core 二进制授予 `CAP_NET_ADMIN`/`CAP_NET_RAW`。

## Profile 下载与更新

远程 profile 会尝试通过已有 TZ 代理和直连下载，只要其中一条路线成功即可，并把成功路线记录为 `download_via`。使用 `tz profile info <name>` 查看该信息；URL 仅在本地 profile 索引中保存，命令输出会隐藏它。

下载请求会根据 family 使用对应客户端的 User-Agent。TZ 只校验并管理原始格式，不会把 Clash YAML 与 sing-box JSON 相互转换。

```bash
tz profile update       # 更新全部远程 profile
tz profile info nano-sb
tz profile remove nano-sb
```

如果直连和当前 TZ 代理都无法下载，先启动一个可用的 TZ profile，或临时启用其他代理后重试。

Mihomo core 目录内的 `Country.mmdb` 和 `GeoSite.dat` 是 GEOIP/GEOSITE 规则数据库，不是需要每位用户单独安装的插件。profile 使用相应规则时，TZ 会把它们复制到运行目录，避免 Mihomo 启动时临时访问 GitHub 下载。

## Shell 补全

当前 shell 临时启用：

```bash
# Bash
eval "$(tz completion generate bash)"

# Zsh
eval "$(tz completion generate zsh)"

# Fish
tz completion generate fish | source
```

要永久启用，把对应命令加入 shell 的启动文件。

## 完整指令

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

## 简洁指令

```bash
tz                 # 状态，并测速当前节点
tz on              # 使用上次的有效 profile 启动并显示状态
tz off              # 停止
tz -l [keyword]     # 节点测速、延迟排序、搜索和选择
tz select           # 当前 family 的 profile 列表和选择
```

## 快捷键

快捷键是完整指令的缩写：

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

使用 `tz --help` 或 `tz <command> --help` 查看参数详情。
