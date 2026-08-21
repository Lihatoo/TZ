# TZ 统一控制指令架构（讨论稿）

状态：Draft 0.1  
范围：Clash/Mihomo、sing-box，以及待确认的 NinjaDesktop Lite 适配器  
参考：`/mnt/data_4t2/lht_self/sing-box-13/sb`

第一版:下面的架构以假设使用.sh调用原二进制内核开发，

## 1. 目标

对用户只暴露一套稳定的 `tz` 指令。切换内核后，用户的日常命令、配置档名称、当前节点、路由模式、TUN、终端代理和系统代理等状态不变；由内核适配器负责生成配置并执行不同的底层指令。

```text
用户 / shell
    |
    v
tz CLI（解析、状态、锁、事务、统一输出）
    |
    +-- clash adapter ------> mihomo/clash + Clash API
    +-- sing-box adapter ---> sing-box + Clash API
    `-- ninja adapter ------> 待探测
```

基本原则：

1. 正式命令统一为 `tz <对象> <动作> [参数]`。
2. 常用操作保留短命令，但短命令只是正式命令的别名。
3. 用户状态由 `tz` 保存，不能以某个内核的运行配置作为唯一状态源。
4. 原始订阅/配置档只读保存；端口、DNS、TUN、绕过规则等系统配置在启动前覆盖合并。
5. 所有内核差异只进入 adapter；主控制逻辑中不散落 `if clash` / `if sing-box`。
6. 不支持的能力必须明确报错并返回非零退出码，不能假装执行成功。
7. 修改内核、配置档、模式、TUN 等操作均先生成并校验配置，再应用；失败时保留旧的可运行状态。

## 2. 完整指令树

我想做的就是普通clash界面的终端操控指令

```text
tz
快速启动
|-- status [--watch]   # 显示整体状态，使用的内核，profile情况，节点，延迟等
|-- start       # 启动，直接启动上次使用的profile，之后立刻执行status，做参考
|-- stop/end        # 停止服务
|-- restart     # 强制完整重启，等效于 stop 后 start
|-- reload      # 重新生成配置并热加载，必要时自动重启
|
|-- list|-l [keyword]       # 列出默认策略组节点并按延迟排序，快速切换节点
|       [--group|-g <name>]
|       [--fresh]  # 这是什么?
|-- use <node> [--group <name>]           # 快速选择节点

整体服务
|-- service  服务,选择节点(对应当前profile），开关等
|   |-- status [--watch]
|   |-- stop/end  # 停止进程
|   |-- restart [profile]  # 重启多用来刷新 bypass.list
|
|-- core
|   |-- list  查看有哪些内核，比如clash ，sing-box，mihomo。用*指出当前内核，选择可换
|   |-- info [name]  当前内核的版本，二进制位置、导入时的url，是否正常启动等信息，或者是指定的
|   |-- add <name> <url>不需要这个我后面会给出内核加入的标准格式，只做上传到网上，下载即可。如果名字重复，可以选择覆盖或者输入新的名称
|   |-- remove <name>  删除对应内核
|   |-- use <name> [--no-start] 切换内核，
|   `-- update [name]  去查看url的内核版本，可选是否更新，否则提示无更新

|-- setting 这个是所有的core共用(配置一次就可以用了）?还是每个core有自己的单独配置(麻烦)
|   |-- list # 列表展示 项目(key）+ 出来。可以选择设置哪一个
|   |-- get <key> # 这个是只展示某个key的配置
|   |-- set <key> <value>  # 这个是单独配置
|   `-- reset [key] # 恢复默认值
|
|-- key 这里展示setting可以打开哪些项目及其配置。下面指出的是默认配置
|   |-- core ： mihomo   # 等效于 core list，
|   |-- sysproxy ： on/off # 系统proxy开启，注意需要识别bypass.list 配置ignore host
|   |-- proxy ： on/off 终端～/.bashrc中的变量配置
|   |-- http-proxy : 127.0.0.1:7892 
|   |-- socks-proxy : 127.0.0.1:7891
|   |-- mixed-proxy : 127.0.0.1:7890 
|   |-- WebUI  ： 127.0.0.1:9189
|   |-- autostart :   true/false

|-- env 环境配置
|   |-- list # 展示出来，有哪些可以配置
|   |-- sysproxy ： on/off # 系统proxy开启，注意需要识别bypass.list 配置ignore host
|   |-- proxy ： on/off 终端～/.bashrc中的变量配置
|   |-- http-proxy : 127.0.0.1:7892 
|   |-- socks-proxy : 127.0.0.1:7891
|   |-- mixed-proxy : 127.0.0.1:7890 
|   |-- WebUI  ： 127.0.0.1:9189

|-- profile  # 由于内核不同profile也不同，所以注意区分。profile是对应core的
|   |-- list # 列出订阅,*指出当前的订阅，注意profile与core对应
|   |-- show [name]
|   |-- add <name> <url-or-file> [--format auto|clash|sing-box]  # 新增订阅，重名选择覆盖(remove后add）还是重命名
|   |-- remove <name> # 删除
|   |-- use <name> [--restart] 
|   |-- current
|   |-- update [name]
|   |-- update-all
|   |-- check [name] [--core <name>]
|   `-- source [name]
|
|-- group   #  profile对应的group可选择切换与node 节点应该在一起
|   |-- list 
|   |-- current [group]
|   `-- select <group> <node>
|-- node 节点
|   |-- list [--group <name>] [--match <keyword>]
|   |-- current [--group <name>]
|   |-- select <node> [--group <name>]
|   `-- test [--group <name>] [--match <keyword>]
|               [--url <url>] [--timeout <ms>] [--select]
|
|-- mode # 这个放在这儿?为什么不放在setting
|   |-- get
|   `-- set rule|global|direct
|
|-- tun
|   |-- status
|   |-- on
|   `-- off
|
|-- proxy
|   |-- status
|   |-- on
|   |-- off
|   |-- env
|   |-- noenv
|   |-- shell-init [bash|zsh|fish]
|   |-- terminal on|off
|   `-- system on|off
|
|-- rule
|   `-- bypass
|       |-- list
|       |-- add <domain-or-cidr>
|       |-- remove <domain-or-cidr>
|       |-- import <file>
|       |-- reset
|       `-- apply
|
|-- connection
|   |-- list
|   |-- close <id>
|   `-- close-all
|
|-- config
|   |-- path
|   |-- show [--effective|--source]
|   |-- build [--core <name>] [--profile <name>]
|   |-- check [--core <name>] [--profile <name>]
|   `-- apply
|
|-- log # 日志没必要吧?
|   `-- show [--lines <n>] [--follow] [--level <level>]
|
|-- diagnose # 这是什么? 
|   |-- run [--full]
|   |-- ports
|   |-- process [pid]
|   |-- network direct|proxy|compare
|   |-- dependencies
|   |-- interfaces
|   |-- dns
|   |-- routes
|   |-- api
|   `-- config
|
|-- api status
|-- api get <path>
`-- completion generate bash|zsh|fish
```





# 文件树

## 





# 具体配置



