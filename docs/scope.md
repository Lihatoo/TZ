# tz v0.1 范围

- 只支持 Linux，同一时间只运行一个当前用户的受管 core，不使用 systemd。
- 支持本地导入 Mihomo 1.19.18（`family=clash`）和 sing-box 1.13.14（`family=sing-box`）。
- 支持本地或 HTTP(S) profile；远程下载在环境代理与直连之间回退，并记录实际成功路径。
- 支持 Clash YAML 与 sing-box JSON 的生成、真实 core 校验、启动、状态、节点选择与测速、重启和安全停止。
- `profile list` 默认跟随当前 core family；`--all` 才跨 family 显示。
- 支持 Bash/Zsh/Fish 终端代理环境输出与 shell hook；支持 GNOME `gsettings` system proxy 和 bypass。
- 支持独立 TUN 开关、能力与权限检查、运行中重启及失败回滚；二进制 capability 由用户显式设置。
- 暂不支持非 GNOME 桌面 system proxy adapter、core 在线安装或自动更新。
