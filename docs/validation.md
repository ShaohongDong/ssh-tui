# 0.1.0 中英文界面本地验收记录

验收日期：2026-10-10。结果：PASS。

以下原始界面验收对应首次提交时的 Linux 交付物与 `e2e-release.json`。本次 Linux/macOS 自动构建改动的验证记录另见文末，两个可执行文件的哈希分别记录。

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

## Linux/macOS 自动构建与发布验证（2026-10-10）

- 新工作流：`.github/workflows/build.yml`，覆盖 Linux x64、macOS Intel、macOS Apple Silicon；分支 Push、PR、手动运行生成 Actions artifacts，匹配 Cargo 版本的 `v*` 标签 Push 自动发布 Release。
- 固定 Rust 1.98.1、Cargo.lock 和官方 Actions 的完整提交 SHA。发布仅使用任务级 `contents: write` 与 `GITHUB_TOKEN`，三个目标全部成功后校验压缩包与 SHA-256，生成 `SHA256SUMS`。
- 本地主机为 Linux x86_64、glibc 2.39；CI Linux 构建基线为 Ubuntu 22.04/glibc 2.35。macOS 部署目标为 13.0，原生 CI 验收系统为 macOS 15。
- `cargo fmt --all --check`、Linux 原生目标的 Clippy、30 项 Rust 测试与 release 构建通过。
- 使用显式 `cargo +1.98.1 check --locked --all-targets --target <target>`，macOS Intel 与 Apple Silicon 两个目标均通过编译检查；该检查不链接可执行文件、不运行 macOS 测试。
- `actionlint 1.7.12` 工作流检查通过；7 项发布脚本测试通过，覆盖三目标架构和文件权限、版本标签不匹配、缺少平台包、校验和不符、额外资产及不安全归档条目。
- 当前原生文件：`target/x86_64-unknown-linux-gnu/release/ssh-tui`，2,685,184 字节；SHA-256 为 `3bc7ad655dedbf1ca7fd5dbef589e97c28e5fee7af946c01092f440099d73b38`。
- 本轮对该文件再次完成英文、中文各 24 项真实交互场景，共 48 项通过；结果位于 `target/ci-e2e-en/`、`target/ci-e2e-zh/`，报告新增平台、架构和语言字段。
- 进程检查改为 Linux/macOS 通用的 `ps` 参数，使用完整命令行；macOS `top` 使用对应输出标记。远程 fixture 与启动包装脚本使用 PATH 上的 Bash，macOS CI 安装现代 Bash 以完整验证括号粘贴。
- Linux 包位于 `dist/packages/ssh-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`，附单独 `.sha256`。打包脚本检查二进制格式、CPU 架构、版本、执行权限，解压后再次运行 `--version` 与 `--help`。

复现本轮交互检查：

```bash
PYTHONPATH=/tmp/ssh-tui-test-deps python3 tests/e2e.py \
  --binary target/x86_64-unknown-linux-gnu/release/ssh-tui \
  --language en --output target/ci-e2e-en
PYTHONPATH=/tmp/ssh-tui-test-deps python3 tests/e2e.py \
  --binary target/x86_64-unknown-linux-gnu/release/ssh-tui \
  --language zh --output target/ci-e2e-zh
python3 -m unittest discover -s tests -p 'test_release.py' -v
python3 scripts/package_release.py package \
  --binary target/x86_64-unknown-linux-gnu/release/ssh-tui \
  --target x86_64-unknown-linux-gnu --output dist/packages --tag v0.1.0
```

本轮未执行 GitHub 远端工作流、macOS 原生运行、签名或 notarization，也未创建标签、提交或推送。macOS 的原生编译和两种语言交互结果须在推送后通过对应 Runner 确认；工作流配置完成与 Linux 本地通过不等于 macOS 已验收。macOS 13/14 和其他生产环境亦未实测。

## 主题扩充与选择器验证（2026-10-11）

结果：PASS，Linux x86_64 本地验证。使用 Rust 1.96.0 和项目内 `.venv` 的 Python 3.11.15。

- 内置主题从六种扩充至十二种，新增 Catppuccin Mocha、One Dark、Monokai、Kanagawa（Wave）、Everforest Dark（Medium）、Rose Pine。原有顺序和 Tokyo Night 默认值保留。
- 选择器增加可见列表行数对应的翻页、首尾跳转，以及包含自定义主题的当前位置/总数。上下和 j/k 保持循环预览；取消恢复、明确保存、写入失败保护和 CLI 临时覆盖通过测试。
- `cargo fmt --all --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked`、`cargo build --release --locked` 全部通过；Rust 测试包含 32 项模块测试和 4 项 CLI 集成测试。
- `.venv/bin/python -m unittest discover -s tests -p 'test_release.py' -v`：7 项打包测试通过。
- 两种语言、80×24 / 120×36 / 48×12 / 24×6 的 88 个视图快照已更新并审阅，包含主题列表首尾。另检查 15 个追加自定义主题的滚动、窄窗口位置提示、全部主题的 UI 对比度和远端显式颜色保持。
- 中英文各 25 项实际交互场景通过，共 50 项；新增场景覆盖全部六种新主题、翻页、首尾、循环、取消、保存重启、CLI 覆盖不落盘，以及切换后活动 SSH 会话继续输入。
- 可执行文件：`target/release/ssh-tui`，SHA-256：`0b63e9a32887c75d3a33a61af0714ed4096984b1b826000fd867b3a7efcd3f7c`。

交互验收使用隔离的 loopback SSH/代理 fixtures，复现命令：

```bash
.venv/bin/python tests/e2e.py --binary target/release/ssh-tui \
  --language en --output target/e2e-theme-en
.venv/bin/python tests/e2e.py --binary target/release/ssh-tui \
  --language zh --output target/e2e-theme-zh
```

报告位于对应目录的 `report.json`，主题列表末尾的实际屏幕文本位于 `theme-picker-last.txt`。本次未运行 macOS；对应原生构建和交互检查仍由 CI 验证。

## v0.1.1 发布前验证（2026-10-11）

使用上述 Linux 环境，将 Cargo.toml、Cargo.lock 的本项目版本同步为 `0.1.1`，并更新 README 当前版本示例。

- 格式检查、Clippy、36 项 Rust 测试、release 构建及 `check-tag --tag v0.1.1` 全部通过；可执行文件输出 `ssh-tui 0.1.1`。
- 使用新版可执行文件重新完成中英文各 25 项实际交互场景，共 50 项通过。报告位于 `target/e2e-v0.1.1-{en,zh}/report.json`。
- 生成并验证 Linux 原生归档至 `dist/v0.1.1-local/`。指定该目录后，全部 28 项 Python 打包与安装测试通过，无跳过项，包含真实包安装与重装。
- 新版本地可执行文件 SHA-256：`1d4fe31851cff8a10d4bd4098615fa2907d8e940b93540e9145042c80e26564c`。

```bash
.venv/bin/python scripts/package_release.py check-tag --tag v0.1.1
.venv/bin/python scripts/package_release.py package \
  --binary target/release/ssh-tui --target x86_64-unknown-linux-gnu \
  --output dist/v0.1.1-local --tag v0.1.1
SSH_TUI_INSTALL_TEST_PACKAGES=dist/v0.1.1-local \
  .venv/bin/python -m unittest discover -s tests -p 'test_*.py' -v
```

正式发布使用 `v0.1.1` 标签触发现有工作流；Linux x64、macOS Intel、macOS Apple Silicon 均通过原生检查后，由工作流发布三个归档及其校验文件。

## v0.1.1 交互验收同步修复（2026-10-11）

首次标签工作流的 macOS Intel 中文验收在语言保存后立即读取旧配置，报 `KeyError: 'language'`；英文验收还出现测试 Bash 回收超时。该工作流未发布 Release。

- 将持久化操作的检查改为等待配置文件实际更新且内容符合预期，超时仍失败。即使内容相同，也要求发生写入，确保 CLI 覆盖测试确实检查了保存后的配置。
- SSH fixture 同时终止进程组和 Bash 本身，等待子进程及输入/服务线程退出。后台清理异常统一在主验收线程报告，避免输出 PASS 后遗漏线程失败。
- 新增四项标准库回归测试，覆盖初始配置缺少语言字段且延迟写入、内容不变的原子保存，以及原进程组存在/不存在时的子进程回收。
- 使用 Python 3.11.15 完成全部 32 项 Python 打包、安装与验收辅助测试，无跳过；Linux 中英文各 25 项交互场景通过，报告位于 `target/e2e-v0.1.1-fix-{en,zh}/report.json`。原生 macOS 结果由修复提交的新工作流验证。
