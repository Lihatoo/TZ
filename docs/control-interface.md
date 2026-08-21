# TZ Control Interface v0.1

TZ 的公开命令围绕用户动作设计。配置文件是可查阅的内部存储格式，修改由固定命令完成并经过校验、锁保护和原子保存。

## 当前指令

```text
tz init
tz status
tz start
tz stop
tz restart
tz list [keyword]
tz node test [keyword] [--url <url>] [--timeout <ms>] [--select]

tz tun status|on|off
tz proxy status|on|off
tz proxy terminal status|on|off
tz proxy system status|on|off
tz proxy env|noenv [bash|zsh|fish]
tz proxy shell-init bash|zsh|fish

tz setting [list]
tz setting get <key>
tz setting set <key> [value]
tz setting reset [key]

tz profile add <name> <url-or-file> --family clash|sing-box
tz profile list [--family clash|sing-box] [--all]
tz profile info <name>
tz profile use [name]
tz profile update
tz profile remove <name>

tz core add <directory>
tz core list
tz core info [name]
tz core use [name]
tz core remove <name>

tz config build|check|show
tz completion generate bash|zsh|fish
```

`profile list` 默认只显示当前 core family，`--all` 显示全部。`profile update` 不接名称，更新所有远程 profile；本地 profile 自动跳过。

`profile list`、`core list` 和节点 `list` 在 TTY 中显示编号、用 `*` 标出当前项并允许直接选择；非交互环境只输出简洁列表。`profile/core use` 保留，作为脚本和明确指定名称的稳定入口。详细来源和路径使用 `info` 查看。

`list/-l` 与 `node test` 都通过当前 core controller API 最多并发测试 8 个节点并按延迟排序。`list/-l` 在 TTY 中允许从测速后的列表选择节点；`node test --select` 自动选择最快的成功节点。默认 URL 为 Google 204，默认超时 1800ms；最新结果保存在 `cache/speedtest/latest.json`。

Mihomo 的 family 固定为 `clash`，sing-box 的 family 固定为 `sing-box`，不能互换。

## 简洁指令

```text
tz                 # status，并测速当前节点
tz on              # 使用上次的可用 profile 启动并显示 status
tz off | tz end    # stop
tz -l [keyword]    # 测速、按延迟排序、搜索和选择
tz select          # profile list
```

节点 keyword 使用不区分大小写的子串匹配。`list/-l` 在 TTY 中可输入测速后列表的编号切换当前节点；选择写入当前 profile，并在下次启动后恢复。

## 快捷键

快捷键是当前指令的命令缩写，完整写法始终可用：

```text
tz st              # status
tz r               # restart
tz set              # setting
tz p                # profile
tz c                # core
tz cfg              # config
tz comp              # completion

tz p a|l|i|u|up|rm
tz c a|l|i|u|rm
```

Tab 补全通过 `tz completion generate <shell>` 生成。例如 Bash 当前会话可执行 `eval "$(tz completion generate bash)"`。

## Proxy 与 TUN

`proxy terminal on|off` 保存终端代理状态。子进程不能直接修改父 shell，因此当前 shell 使用 `eval "$(tz proxy env)"` 或 `eval "$(tz proxy noenv)"`；长期使用把 `eval "$(tz proxy shell-init bash)"` 加入对应 shell 启动文件。Fish 和 Zsh 使用各自的 shell 参数。

`proxy system on|off` 使用 GNOME `gsettings` 设置 HTTP、HTTPS、SOCKS 和 ignore-hosts，端口与 bypass 均来自 TZ 配置。开启前要求 core 正在运行，避免桌面流量指向空端口。TZ 会先私有备份原桌面代理，关闭或应用失败时逐项恢复；未由 TZ 开启时，`system off` 不修改桌面设置。`proxy on|off` 同时控制 terminal 和 system。

`tun on|off` 检查当前 core 的 TUN capability 和 `/dev/net/tun`。开启还要求受管 core 二进制具有 `CAP_NET_ADMIN`；缺少时打印对应的 `sudo setcap cap_net_admin,cap_net_raw+ep ...` 命令。运行中的切换会安全重启，失败时恢复原状态。

## Profile 下载

URL 只允许 HTTP/HTTPS，并校验 DNS 与重定向目标。下载按 family 使用对应 provider User-Agent：Clash 对齐 `mh`，sing-box 对齐 `sb`，以支持服务端按客户端返回不同格式。下载优先尝试环境代理，失败后回退直连，只要一种路径成功即完成添加或更新。实际成功路径保存为 profile 的 `download_via=proxy|direct`，可用 `tz profile info <name>` 查看；错误信息不会打印订阅 token。

## 运行闭环

`config build/check` 根据当前 core family 生成 Clash YAML 或 sing-box JSON，并调用 core manifest 的 check 命令。Mihomo profile 引用 GEOIP/GEOSITE 时，builder 从 core 包按需复制 `Country.mmdb` 和 `GeoSite.dat` 到独立工作目录；标准 core 包已携带这两份规则数据库，用户无需手工处理。自定义 core 包缺失时会明确提示，可先启用其他代理取得资源后补入 core 包。

`start/on` 读取当前 family 上次选择且 source 可用的 profile，在校验通过后启动受管进程、等待 API 并显示 status；没有可用 profile 时提示运行 `tz profile list`。`stop` 在发送信号前核对进程用户和 `/proc/<pid>/exe`；`status/tz` 简洁显示 core、profile、服务 PID，并实时测试当前节点延迟，失败时才回退显示缓存结果。

当前尚未开放的是 core 在线下载与自动更新。System proxy 的 v0.1 平台适配范围是 GNOME；其他 Linux 桌面环境后续增加 adapter。
