# TZ Core Package v1

本规范定义 TZ 如何发现、校验和调用本地代理内核。core 包包含运行契约、二进制和该 core 固定的只读运行资源，不包含 profile、用户配置、secret、日志、PID 或缓存。

## 目录结构

稳定槽位名推荐使用 `mihomo`、`sing-box`：

```text
<data_dir>/cores/
└── mihomo/
    ├── core.toml
    ├── mihomo
    ├── Country.mmdb # Mihomo GEOIP 规则数据库
    ├── GeoSite.dat  # Mihomo GEOSITE 规则数据库
    ├── LICENSE      # 可选
    ├── NOTICE       # 可选
    └── README.md    # 可选，TZ 不解析
```

目录名必须与 `core.name` 完全一致。完整版本保存在 `core.version`；需要多版本并存时使用完整槽位名，例如 `mihomo-1.19.14`。

Mihomo 标准包固定携带 `Country.mmdb` 和 `GeoSite.dat`。生成配置实际引用 GEOIP/GEOSITE 时，TZ 才把缺失资源复制到 `state/runtime/<core>/`；用户无需逐个 profile 手工下载。自制 Mihomo core 包也应携带这两个文件，否则 TZ 会在 build/check 阶段给出明确提示。

## Manifest

```toml
schema_version = 1

[core]
name = "mihomo"
family = "clash"
version = "1.19.18"
binary = "mihomo"
os = "linux"
arch = "x86_64"

[runtime]
entrypoint = "config.yaml"
format = "yaml"

[capabilities.config]
mixed_proxy = true
http_proxy = true
socks_proxy = true
api = true
dns = true
tun = true

[commands.start]
args = ["-d", "{workdir}", "-f", "{config}"]

[commands.check]
args = ["-t", "-d", "{workdir}", "-f", "{config}"]

[commands.version]
args = ["-v"]
```

字段约束：

- schema 当前仅支持 `1`，未知字段会被拒绝。
- name 只允许 ASCII 字母、数字、点、下划线和连字符。
- family/format 当前只允许 `clash/yaml` 和 `sing-box/json`。
- os/arch 必须等于当前运行平台的 Rust target 常量。
- binary 与 entrypoint 必须是单个相对文件名，禁止绝对路径和 `..`。
- binary 必须是普通可执行文件。
- start 必填；check、version、reload 可选。命令存在即表示支持对应动作。
- 参数只支持 `{config}` 和 `{workdir}`；TZ 直接执行 binary，不经过 shell。

## 安装方式

手工复制是标准方式，无注册数据库：

```bash
cp -a ./mihomo <data_dir>/cores/mihomo
chmod +x <data_dir>/cores/mihomo/mihomo
tz core list
```

本地便捷导入：

```bash
tz core add ./mihomo
tz core info mihomo
tz core use mihomo
tz core remove mihomo
```

`core add` 只接收本地目录，不接收 URL。它会拒绝符号链接和特殊文件，复制到 staging，重新校验后原子移动；重名直接拒绝。成功后不自动选择或启动。

`core remove` 在受管进程运行时拒绝。服务停止时可以删除当前 core，并原子清空 current，同时清理同名 generated/runtime 派生目录。

## 网络分发

网络下载不属于 `core add`。未来 `core install` 必须使用可信 registry 提供的外部 SHA256，且 URL 只能是 HTTP/HTTPS；请求和每次重定向前必须拒绝 localhost、环回、私有和保留地址。
