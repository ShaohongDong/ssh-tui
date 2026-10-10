# 0.1.0 中英文界面本地验收记录

验收日期：2026-10-10。结果：PASS。

## 环境与交付物

- Linux x86_64；Rust 1.98.1 stable；系统 OpenSSH 客户端 9.6p1。
- 可执行文件：`dist/ssh-tui`，版本 `ssh-tui 0.1.0`，2,685,592 字节，约 2.6 MiB。
- ELF 动态依赖为系统 glibc、libgcc_s；运行还需要 `ssh` 命令和交互终端。
- SHA-256：`5a8ea241b5a49e6f12ba6c72478fce37d57b3215ce5ac25e68ea2978986f73b2`。
- 使用交付目录中的可执行文件再次完成全部交互验收，内部 ProxyCommand 成功定位同一个可执行文件。

## 自动检查

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --check` | PASS |
| `cargo clippy --all-targets -- -D warnings` | PASS |
| `cargo test --locked` | PASS，27 项模块测试 + 3 项 CLI/示例集成测试 |
| `cargo build --release --locked` | PASS |

模块测试覆盖控制字节、旧终端 Ctrl+] 别名、方向键模式、括号粘贴、跨数据块 UTF-8 和 ANSI、光标查询、备用屏幕、尺寸变化、主题颜色、配置往返及文件权限、配置错误保护、代理响应分段和 SSH banner 合并、认证头、错误状态、响应头限制与握手超时。CLI 检查参数冲突、英文帮助、默认语言、配置语言、命令行覆盖、启动错误的语言，以及交付的配置和自定义主题能够加载。新增模块检查旧配置默认英文、两种语言保存及无效值保护、选择器取消和原页面恢复、写入失败保护、其他设置保存不泄漏语言覆盖，以及本地错误链保留外部诊断原文。英文换行检查单词边界、长词和 Unicode 字素。

UI 验收覆盖两种语言、四种尺寸的 80 个渲染视图：80×24、120×36、48×12、24×6，检查空列表、两条连接、大量长名称、代理、长输入、确认框、帮助首尾、主题预览和语言选择器。快照位于 `tests/snapshots/`，正常测试只读对照。另检查各内置主题和不同背景上的 UI 文字至少 4.5:1 对比度、真实单元格颜色、24 个标签中的活动标签可见性、窄窗口全部字段的输入光标、旧主题和可选 UI 角色兼容，以及 1×1 等极小窗口渲染稳定性。预览图见 [ui-design.md](ui-design.md)。

## 两种语言各 24 项实际交互场景

执行命令：

```bash
PYTHONPATH=/tmp/ssh-tui-test-deps python3 tests/e2e.py \
  --binary dist/ssh-tui --language en --output target/e2e-en
PYTHONPATH=/tmp/ssh-tui-test-deps python3 tests/e2e.py \
  --binary dist/ssh-tui --language zh --output target/e2e-zh
```

| 场景 | 结果 |
| --- | --- |
| 首次主机指纹确认与公钥登录 | PASS |
| 远程 PTY 初始尺寸与窗口缩放 | PASS |
| Ctrl+C 中断远程 sleep，客户端继续运行 | PASS |
| 双前缀及未知前缀组合逐字节转发 | PASS |
| 粘贴中的前缀不触发本地命令，18 KB 粘贴可完成 | PASS |
| 中文宽字符及 ANSI 颜色 | PASS |
| 语言预览、取消、保存、重启读取、CLI 覆盖不落盘、活动连接连续性，以及普通 l 转发 | PASS，en/zh |
| 主题预览、取消恢复和保存 | PASS |
| HTTP CONNECT 登录、后台输出和标签输入隔离 | PASS |
| 滚屏及返回实时画面 | PASS |
| top、less、vim.tiny 全屏交互与文件编辑 | PASS |
| 断开后保留画面，手动重连 | PASS |
| 正常退出恢复终端并关闭连接 | PASS |
| Basic 代理认证、密码不回显、不保存 | PASS |
| 错误代理密码产生 407，无直连回退 | PASS |
| 目标被代理拒绝产生 403，无直连回退 | PASS |
| 有效 OpenSSH ProxyJump 与应用代理冲突拒绝 | PASS |
| IPv6 代理及 SIGTERM 后终端恢复 | PASS |
| 连接表单新增、搜索、编辑、删除及持久化，以及 80×24 紧凑弹层、48×12 错误详情/帮助滚动/主题取消 | PASS |
| 代理表单新增、设默认、禁用、编辑、删除 | PASS |
| 快速连接不保存，关闭确认取消与确认 | PASS |
| SSH 密码认证 | PASS |
| 已知主机密钥变化拒绝连接 | PASS |
| 无残留 SSH 或内部代理子进程 | PASS |

机器可读结果见 [e2e-release.json](e2e-release.json)。本轮英文和中文的终端、连接列表屏幕文本分别保留在 `target/e2e-en/`、`target/e2e-zh/`。全部表中场景均在两种语言下通过，共 48 次场景验收。

## 语言行为

界面默认英文。`--language en|zh` 优先于配置 `language = "en"|"zh"`，旧配置缺少该字段时使用英文，不读取系统语言作为默认值。连接与代理列表按 `l`，远程终端使用 `Ctrl+] l`；上下键预览，Enter 保存，Esc 恢复并返回原页面。CLI 语言覆盖不因其他设置保存而落盘。

应用自身的菜单、帮助、确认、状态、通知、错误及代理密码提示支持中英文。用户名称、主题名称、远端输出和系统/第三方原始诊断保持原样。代理辅助进程使用连接建立时的语言；已有终端缓冲不重写，重连使用当前语言。CLI 帮助固定英文。

## 验收边界

客户端是真实系统 OpenSSH；服务端是测试脚本启动的临时 Paramiko SSH 服务，远程 shell 使用独立 PTY。HTTP 代理也由脚本启动，测试 IPv4/IPv6 回环地址、无认证/Basic、拒绝状态及响应头与 SSH banner 同包。使用临时密钥、隔离 SSH 配置和 known_hosts，不读取个人凭据、不连接真实服务器。

此次结果证明本地功能与交互链路通过，尚未对真实生产服务器、企业代理或其他外层终端进行验收。HTTPS/HTTP2、SOCKS、NTLM/Kerberos、HTTP 代理与 SSH 跳板机组合、分屏及 SFTP 属于首版之外的功能。
