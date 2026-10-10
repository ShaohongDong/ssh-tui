//! Application-owned text is localized at display time; external text is opaque.
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Zh,
}
impl Language {
    pub const ALL: [Self; 2] = [Self::En, Self::Zh];
    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Zh => "zh",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Zh => "中文",
        }
    }
    pub fn text(self, key: Text) -> &'static str {
        let (en, zh) = key.pair();
        match self {
            Self::En => en,
            Self::Zh => zh,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Text {
    Saved,
    NoDetails,
    PortNumber,
    ProxyReferenced,
    HelpTitle,
    DetailsTitle,
    ConfirmQuit,
    ConfirmClose,
    ConfirmDeleteProfile,
    ConfirmDeleteProxy,
    Confirm,
    Prefix,
    Help,
    Connect,
    Tabs,
    Theme,
    Details,
    Back,
    Scrollback,
    Page,
    Proxy,
    Quick,
    Default,
    DisableDefault,
    Change,
    Clear,
    Preview,
    Save,
    Restore,
    Scroll,
    Cancel,
    PrefixDetails,
    LocalDetails,
    DisconnectedHint,
    Connections,
    Count,
    Search,
    SshConfig,
    NoConnections,
    Add,
    Edit,
    Delete,
    More,
    Proxies,
    DefaultOff,
    NoAuth,
    NoProxies,
    Disable,
    QuickTitle,
    EditConnection,
    Name,
    Host,
    User,
    Port,
    Identity,
    Jump,
    HostHelp,
    SshHelp,
    RouteHelp,
    NameHelp,
    EditProxy,
    HttpUrl,
    BasicUser,
    ProxyHelp,
    Empty,
    ThemeTitle,
    PreviewHints,
    ScrollHint,
    Language,
    LanguageHelp,
    StartSession,
    Running,
    InputFailed,
    Disconnected,
    SessionDisconnected,
    InputClosed,
    ReadPath,
    ThemeMissing,
    NeedTerminal,
    ConnectionNameEmpty,
    PortRange,
    IdentityInvalid,
    ProxyNameEmpty,
    ProxyUserInvalid,
    ConfigToml,
    ReadConfig,
    ScrollbackLimit,
    DuplicateProxy,
    DuplicateConnection,
    ProxyJumpConflict,
    ProxyMissing,
    HomeMissing,
    CreateConfigDir,
    SaveConfig,
    ProxyNameInvalid,
    ProxyUrlInvalid,
    HttpOnly,
    ProxyCredentials,
    ProxyUrlParts,
    ProxyHostMissing,
    ProxyPortInvalid,
    ProxyPortZero,
    TargetPortZero,
    DnsTimeout,
    DnsFailed,
    ProxyConnect,
    ProxyPasswordRequired,
    SendConnect,
    ConnectFailed,
    HeaderLimit,
    ResponseIncomplete,
    StatusMissing,
    TooManyInterim,
    ProxyAuthFailed,
    ProxyDenied,
    ProxyStatus,
    HandshakeTimeout,
    ReadResponse,
    ProxyClosed,
    PasswordTerminal,
    ReadTerminal,
    DisableEcho,
    PasswordPrompt,
    PasswordCancelled,
    PasswordTooLong,
    HostInvalid,
    HostCharacters,
    Ipv6Invalid,
    UserInvalid,
    UserEmpty,
    JumpPortInvalid,
    JumpPortZero,
    HttpJumpConflict,
    SshMissing,
    SshEffective,
    SshRouteConflict,
    SshHostMissing,
    SshPortMissing,
    ExeUtf8,
    ColorInvalid,
    ThemeNameEmpty,
    ThemePalette,
    ThemeToml,
    ThemeInvalid,
    DuplicateTheme,
    HelpText,
}
impl Text {
    fn pair(self) -> (&'static str, &'static str) {
        match self {
            Self::Saved => ("Settings saved", "配置已保存"),
            Self::NoDetails => ("No error details", "没有错误信息"),
            Self::PortNumber => ("Port must be a number", "端口必须是数字"),
            Self::ProxyReferenced => (
                "Proxy is referenced by a connection; update its route first",
                "代理被连接引用，请先修改连接配置",
            ),
            Self::HelpTitle => ("Keyboard help", "快捷键帮助"),
            Self::DetailsTitle => ("Details", "详细信息"),
            Self::ConfirmQuit => (
                "Quit and close all active connections?",
                "退出并关闭全部活动连接？",
            ),
            Self::ConfirmClose => ("Close the current SSH connection?", "关闭当前 SSH 连接？"),
            Self::ConfirmDeleteProfile => ("Delete this connection profile?", "删除此连接配置？"),
            Self::ConfirmDeleteProxy => ("Delete this proxy profile?", "删除此代理配置？"),
            Self::Confirm => ("OK", "确认"),
            Self::Prefix => ("Prefix", "前缀"),
            Self::Help => ("Help", "帮助"),
            Self::Connect => ("Open", "连接"),
            Self::Tabs => ("Tabs", "标签"),
            Self::Theme => ("Theme", "主题"),
            Self::Details => ("Details", "详情"),
            Self::Back => ("Back", "返回"),
            Self::Scrollback => ("History", "滚屏浏览"),
            Self::Page => ("Page", "翻页"),
            Self::Proxy => ("Proxy", "代理"),
            Self::Quick => ("Quick", "快连"),
            Self::Default => ("Default", "默认"),
            Self::DisableDefault => ("No default", "关闭默认"),
            Self::Change => ("Next", "切换"),
            Self::Clear => ("Clear", "清空"),
            Self::Preview => ("Preview", "预览"),
            Self::Save => ("Save", "保存"),
            Self::Restore => ("Restore", "恢复"),
            Self::Scroll => ("Scroll", "滚动"),
            Self::Cancel => ("Cancel", "取消"),
            Self::PrefixDetails => (" · Ctrl+] ! Details", " · Ctrl+] ! 详情"),
            Self::LocalDetails => (" · F2 Details", " · F2 详情"),
            Self::DisconnectedHint => ("Disconnected · r reconnect", "已断开 · r 重连"),
            Self::Connections => ("Connections", "连接列表"),
            Self::Count => ("{0} items", "{0} 项"),
            Self::Search => ("/ Search connections", "/ 搜索连接"),
            Self::SshConfig => ("SSH config", "SSH 配置"),
            Self::NoConnections => (
                "No matches · a add / c quick",
                "暂无匹配连接 · a 新增 / c 快连",
            ),
            Self::Add => ("Add", "新增"),
            Self::Edit => ("Edit", "编辑"),
            Self::Delete => ("Delete", "删除"),
            Self::More => ("More", "更多"),
            Self::Proxies => ("HTTP proxies", "HTTP 代理配置"),
            Self::DefaultOff => ("Default off", "默认关闭"),
            Self::NoAuth => ("No auth", "无认证"),
            Self::NoProxies => ("No proxies · a add", "暂无代理 · a 新增"),
            Self::Disable => ("Disable", "禁用"),
            Self::QuickTitle => ("Quick connection (unsaved)", "快速连接（不保存）"),
            Self::EditConnection => ("Edit connection", "编辑连接"),
            Self::Name => ("Name", "名称"),
            Self::Host => ("Host / alias", "主机 / 别名"),
            Self::User => ("Username", "用户名"),
            Self::Port => ("Port", "端口"),
            Self::Identity => ("Identity file", "密钥路径"),
            Self::Jump => ("Jump host", "跳板机"),
            Self::HostHelp => (
                "Hostname, IP address, or ~/.ssh/config alias",
                "支持主机地址或 ~/.ssh/config 中的别名",
            ),
            Self::SshHelp => (
                "Leave empty to use OpenSSH configuration",
                "留空沿用 OpenSSH 配置",
            ),
            Self::RouteHelp => (
                "Proxy: inherit / direct / saved proxy name",
                "代理：inherit / direct / 已配置的代理名称",
            ),
            Self::NameHelp => ("Name used in lists and tabs", "连接名称用于列表与标签"),
            Self::EditProxy => ("Edit HTTP proxy", "编辑 HTTP 代理"),
            Self::HttpUrl => ("HTTP URL", "HTTP 地址"),
            Self::BasicUser => ("Basic username", "Basic 用户名"),
            Self::ProxyHelp => (
                "HTTP CONNECT only · password prompted on connect",
                "仅支持 HTTP CONNECT · 密码连接时输入",
            ),
            Self::Empty => ("Empty", "留空"),
            Self::ThemeTitle => ("Theme preview", "主题预览"),
            Self::PreviewHints => ("Secondary text · keyboard hints", "辅助文字 · 快捷键提示"),
            Self::ScrollHint => ("↑↓ Scroll", "↑↓ 滚动"),
            Self::Language => ("Language", "语言"),
            Self::LanguageHelp => (
                "Preview language · Enter saves / Esc restores",
                "预览语言 · Enter 保存 / Esc 恢复",
            ),
            Self::StartSession => ("Could not start SSH session", "无法启动 SSH 会话"),
            Self::Running => ("SSH process running", "SSH 进程运行"),
            Self::InputFailed => ("Input failed: {0}", "输入失败: {0}"),
            Self::Disconnected => ("Disconnected / {0} / r reconnect", "已断开 / {0} / r 重连"),
            Self::SessionDisconnected => (
                "Session disconnected; press r to reconnect",
                "会话已断开，请按 r 重连",
            ),
            Self::InputClosed => ("Session input is closed", "会话输入已关闭"),
            Self::ReadPath => ("Reading {0}", "读取 {0}"),
            Self::ThemeMissing => (
                "Theme not found; check configuration or --theme",
                "主题不存在，请检查配置或 --theme",
            ),
            Self::NeedTerminal => ("Run in an interactive terminal", "请在交互式终端中运行"),
            Self::ConnectionNameEmpty => ("Connection name cannot be empty", "连接名称不能为空"),
            Self::PortRange => (
                "Port must be between 1 and 65535",
                "端口必须在 1–65535 之间",
            ),
            Self::IdentityInvalid => (
                "Identity path contains invalid characters",
                "密钥路径包含非法字符",
            ),
            Self::ProxyNameEmpty => ("Proxy name cannot be empty", "代理名称不能为空"),
            Self::ProxyUserInvalid => (
                "Proxy username contains invalid characters",
                "代理用户名包含非法字符",
            ),
            Self::ConfigToml => ("Invalid configuration TOML", "配置 TOML 格式错误"),
            Self::ReadConfig => ("Could not read configuration", "无法读取配置"),
            Self::ScrollbackLimit => (
                "Scrollback cannot exceed 100000 lines",
                "滚屏历史不能超过 100000 行",
            ),
            Self::DuplicateProxy => ("Duplicate proxy name: {0}", "代理名称重复: {0}"),
            Self::DuplicateConnection => ("Duplicate connection name: {0}", "连接名称重复: {0}"),
            Self::ProxyJumpConflict => (
                "HTTP proxy and SSH jump host cannot be combined",
                "HTTP 代理与跳板机不能同时使用",
            ),
            Self::ProxyMissing => ("Proxy not found: {0}", "代理不存在: {0}"),
            Self::HomeMissing => (
                "HOME is unset; specify --config",
                "HOME 未设置，请指定 --config",
            ),
            Self::CreateConfigDir => (
                "Could not create configuration directory",
                "无法创建配置目录",
            ),
            Self::SaveConfig => ("Could not save configuration", "无法保存配置"),
            Self::ProxyNameInvalid => (
                "Proxy name contains invalid characters",
                "代理名称包含非法字符",
            ),
            Self::ProxyUrlInvalid => (
                "Invalid proxy URL; use http://host:port",
                "代理地址格式错误，应为 http://主机:端口",
            ),
            Self::HttpOnly => (
                "Only http:// proxies are supported",
                "首版仅支持 http:// 代理",
            ),
            Self::ProxyCredentials => (
                "Do not embed credentials in the proxy URL; enter the username separately",
                "代理地址不得内嵌认证信息，请单独填写用户名",
            ),
            Self::ProxyUrlParts => (
                "Proxy URL cannot contain a path, query, or fragment",
                "代理地址不能包含路径、查询或片段",
            ),
            Self::ProxyHostMissing => ("Proxy URL has no host", "代理地址缺少主机"),
            Self::ProxyPortInvalid => ("Invalid proxy port", "代理端口无效"),
            Self::ProxyPortZero => ("Proxy port cannot be 0", "代理端口不能为 0"),
            Self::TargetPortZero => ("Target port cannot be 0", "目标端口不能为 0"),
            Self::DnsTimeout => ("Proxy DNS lookup timed out", "代理 DNS 解析超时"),
            Self::DnsFailed => ("Proxy DNS lookup failed", "代理 DNS 解析失败"),
            Self::ProxyConnect => (
                "Could not connect to HTTP proxy (refused or timed out)",
                "无法连接 HTTP 代理（拒绝连接或超时）",
            ),
            Self::ProxyPasswordRequired => ("Proxy requires a password", "代理需要密码"),
            Self::SendConnect => ("Could not send CONNECT request", "发送 CONNECT 请求失败"),
            Self::ConnectFailed => ("HTTP CONNECT failed", "HTTP CONNECT 失败"),
            Self::HeaderLimit => (
                "Proxy response headers exceed 64 KiB",
                "代理响应头超过 64 KiB",
            ),
            Self::ResponseIncomplete => ("Incomplete proxy response", "代理响应不完整"),
            Self::StatusMissing => ("Proxy response has no status code", "代理响应缺少状态码"),
            Self::TooManyInterim => ("Too many interim proxy responses", "代理临时响应过多"),
            Self::ProxyAuthFailed => (
                "Proxy authentication failed (407); check credentials; only Basic is supported",
                "代理认证失败 (407)，请检查用户名密码；首版仅支持 Basic",
            ),
            Self::ProxyDenied => (
                "Proxy denied target host or port (403)",
                "代理拒绝目标主机或端口 (403)",
            ),
            Self::ProxyStatus => ("Proxy returned HTTP {0}", "代理返回 HTTP {0}"),
            Self::HandshakeTimeout => ("CONNECT handshake timed out", "CONNECT 握手超时"),
            Self::ReadResponse => (
                "Could not read proxy response or timed out",
                "读取代理响应失败或超时",
            ),
            Self::ProxyClosed => (
                "Proxy disconnected before completing its response",
                "代理在响应完成前断开",
            ),
            Self::PasswordTerminal => (
                "Proxy password input requires a controlling terminal",
                "代理密码输入需要控制终端",
            ),
            Self::ReadTerminal => ("Could not read terminal mode", "无法读取终端模式"),
            Self::DisableEcho => ("Could not disable password echo", "无法关闭密码回显"),
            Self::PasswordPrompt => ("HTTP proxy password: ", "HTTP 代理密码: "),
            Self::PasswordCancelled => ("Proxy password input cancelled", "代理密码输入取消"),
            Self::PasswordTooLong => ("Proxy password is too long", "代理密码过长"),
            Self::HostInvalid => ("Invalid host or SSH alias", "主机或 SSH 别名无效"),
            Self::HostCharacters => (
                "Host must use hostname, IP, or safe SSH alias characters",
                "主机只能包含域名、IP 或安全的 SSH 别名字符",
            ),
            Self::Ipv6Invalid => ("Invalid IPv6 address", "IPv6 地址无效"),
            Self::UserInvalid => ("Username contains invalid characters", "用户名包含非法字符"),
            Self::UserEmpty => ("Username cannot be empty", "用户名不能为空"),
            Self::JumpPortInvalid => ("Invalid jump-host port", "跳板机端口无效"),
            Self::JumpPortZero => ("Jump-host port cannot be 0", "跳板机端口不能为 0"),
            Self::HttpJumpConflict => (
                "HTTP proxy and SSH jump host cannot be combined",
                "HTTP 代理与 SSH 跳板机不能同时使用",
            ),
            Self::SshMissing => (
                "Could not execute ssh; install an OpenSSH client",
                "无法执行系统 ssh，请安装 OpenSSH 客户端",
            ),
            Self::SshEffective => (
                "Could not resolve effective SSH configuration",
                "无法解析有效 SSH 配置",
            ),
            Self::SshRouteConflict => (
                "HTTP proxy conflicts with SSH {0}; use an alias without that route",
                "HTTP 代理与 SSH 配置中的 {0} 冲突，请使用不含该路由的别名",
            ),
            Self::SshHostMissing => ("SSH configuration has no hostname", "SSH 配置缺少主机"),
            Self::SshPortMissing => ("SSH configuration has no port", "SSH 配置缺少端口"),
            Self::ExeUtf8 => (
                "Executable path must be UTF-8",
                "可执行文件路径必须是 UTF-8",
            ),
            Self::ColorInvalid => ("Color must be #RRGGBB: {0}", "颜色必须是 #RRGGBB: {0}"),
            Self::ThemeNameEmpty => ("Theme name cannot be empty", "主题名称不能为空"),
            Self::ThemePalette => ("Theme requires 16 ANSI colors", "主题需要 16 个 ANSI 颜色"),
            Self::ThemeToml => ("Invalid theme TOML: {0}", "主题格式错误: {0}"),
            Self::ThemeInvalid => ("Invalid theme: {0}", "主题无效: {0}"),
            Self::DuplicateTheme => ("Duplicate theme name: {0}", "主题名称重复: {0}"),
            Self::HelpText => (
                "Remote terminal\nCtrl+C / Ctrl+D / Ctrl+Z / Ctrl+L / Tab / Esc / arrows are forwarded\n\nLocal prefix Ctrl+] followed by\nc connections · n/p tabs · 1–9 select tab\nt themes · l language · [ history · x close · q quit · ! details · ? help\nCtrl+] again sends the prefix; Esc cancels; unknown combinations are forwarded\n\nConnections\nEnter open · a add · e edit · d delete · c quick connection\n/ search · p proxies · t themes · l language · Tab return to session · q quit\n\nHTTP proxies\na add · e edit · d delete · Enter default · 0 disable default · l language\nHTTP CONNECT without auth / Basic; passwords are not saved\nHTTP proxy and SSH jump host cannot be combined\n\nForms\nTab/arrows change field · Left/Right/Home/End edit · Ctrl+U clear\nEnter OK · Esc cancel · F2 full error details\n\nHistory\nArrows / PageUp / PageDown / Home / End · Esc return\n\nThemes and language\nArrows preview · Enter save · Esc restore\n\nTabs: × disconnected · • new background output\nSSH process running means the local process is alive;\nauthentication prompts are in the remote terminal.",
                "远程终端\nCtrl+C / Ctrl+D / Ctrl+Z / Ctrl+L / Tab / Esc / 方向键直接转发\n\n本地前缀 Ctrl+] 后\nc 连接列表 · n/p 切换标签 · 1–9 指定标签\nt 主题 · l 语言 · [ 滚屏 · x 关闭 · q 退出 · ! 详情 · ? 帮助\n再按 Ctrl+] 转发前缀；Esc 取消；未知组合转发远程\n\n连接列表\nEnter 连接 · a 新增 · e 编辑 · d 删除 · c 快速连接\n/ 搜索 · p 代理 · t 主题 · l 语言 · Tab 返回会话 · q 退出\n\nHTTP 代理\na 新增 · e 编辑 · d 删除 · Enter 默认 · 0 关闭默认\nl 切换语言\n支持 HTTP CONNECT 无认证 / Basic；密码不保存\nHTTP 代理与 SSH 跳板机二选一\n\n表单\nTab/上下 切换 · 左右/Home/End 编辑 · Ctrl+U 清空\nEnter 确认 · Esc 取消 · F2 查看完整错误\n\n滚屏\n方向键 / PageUp / PageDown / Home / End · Esc 返回\n\n主题\n上下预览 · Enter 保存 · Esc 恢复\n\n标签状态：× 已断开 · • 后台有新输出\nSSH 进程运行表示本地进程存活，认证提示见远程终端",
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Message {
    pub key: Text,
    args: Vec<String>,
}
impl Message {
    pub fn plain(key: Text) -> Self {
        Self {
            key,
            args: Vec::new(),
        }
    }
    pub fn arg(key: Text, value: impl ToString) -> Self {
        Self {
            key,
            args: vec![value.to_string()],
        }
    }
    pub fn render(&self, language: Language) -> String {
        // Substitute once, so braces inside user input remain literal.
        let template = language.text(self.key);
        let mut result = String::new();
        let mut rest = template;
        while let Some(index) = rest.find("{0}") {
            result.push_str(&rest[..index]);
            result.push_str(self.args.first().map_or("{0}", String::as_str));
            rest = &rest[index + 3..];
        }
        result.push_str(rest);
        result
    }
}
impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render(Language::En))
    }
}
impl Error for Message {}

#[derive(Debug)]
struct ContextError {
    message: Message,
    source: anyhow::Error,
}
impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(f)
    }
}
impl Error for ContextError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}
/// Preserve message keys in context chains instead of eagerly formatting them.
pub trait Context<T> {
    fn context(self, message: Message) -> anyhow::Result<T>;
    fn with_context(self, message: impl FnOnce() -> Message) -> anyhow::Result<T>;
}
impl<T, E: Into<anyhow::Error>> Context<T> for Result<T, E> {
    fn context(self, message: Message) -> anyhow::Result<T> {
        self.with_context(|| message)
    }
    fn with_context(self, message: impl FnOnce() -> Message) -> anyhow::Result<T> {
        self.map_err(|source| {
            anyhow::Error::new(ContextError {
                message: message(),
                source: source.into(),
            })
        })
    }
}
impl<T> Context<T> for Option<T> {
    fn context(self, message: Message) -> anyhow::Result<T> {
        self.with_context(|| message)
    }
    fn with_context(self, message: impl FnOnce() -> Message) -> anyhow::Result<T> {
        self.ok_or_else(|| anyhow::Error::new(message()))
    }
}
#[derive(Clone, Debug)]
enum Part {
    Local(Message),
    External(String),
}
#[derive(Clone, Debug)]
pub struct Diagnostic(Vec<Part>);
impl Diagnostic {
    pub fn capture(error: &anyhow::Error) -> Self {
        Self(
            error
                .chain()
                .map(|part| {
                    if let Some(context) = part.downcast_ref::<ContextError>() {
                        Part::Local(context.message.clone())
                    } else if let Some(message) = part.downcast_ref::<Message>() {
                        Part::Local(message.clone())
                    } else {
                        Part::External(part.to_string())
                    }
                })
                .collect(),
        )
    }
    pub fn render(&self, language: Language) -> String {
        self.0
            .iter()
            .map(|part| match part {
                Part::Local(message) => message.render(language),
                Part::External(text) => text.clone(),
            })
            .collect::<Vec<_>>()
            .join(": ")
    }
}
impl From<Message> for Diagnostic {
    fn from(message: Message) -> Self {
        Self(vec![Part::Local(message)])
    }
}
impl From<String> for Diagnostic {
    fn from(text: String) -> Self {
        Self(vec![Part::External(text)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostic_keeps_keys_and_external_values() {
        let error = Err::<(), _>(std::io::Error::other("external {0} 中文"))
            .context(Message::plain(Text::ReadConfig))
            .unwrap_err();
        let error = Err::<(), _>(error)
            .context(Message::arg(Text::ReadPath, "/tmp/{0}/中文"))
            .unwrap_err();
        let diagnostic = Diagnostic::capture(&error);
        assert_eq!(
            diagnostic.render(Language::En),
            "Reading /tmp/{0}/中文: Could not read configuration: external {0} 中文"
        );
        assert_eq!(
            diagnostic.render(Language::Zh),
            "读取 /tmp/{0}/中文: 无法读取配置: external {0} 中文"
        );
    }
    #[test]
    fn languages_round_trip_and_reject_unknown_values() {
        #[derive(Serialize, Deserialize)]
        struct Test {
            language: Language,
        }
        for language in Language::ALL {
            let encoded = toml::to_string(&Test { language }).unwrap();
            assert_eq!(toml::from_str::<Test>(&encoded).unwrap().language, language);
        }
        assert!(toml::from_str::<Test>("language = 'fr'").is_err());
    }
}
