use crate::config::{Profile, Proxy};
use crate::i18n::{Context, Language, Message, Text};
use anyhow::{Result, ensure};
use portable_pty::CommandBuilder;
use std::process::Command;

pub fn validate_host(host: &str) -> Result<()> {
    ensure!(
        !host.is_empty() && host.len() <= 253 && !host.starts_with('-'),
        Message::plain(Text::HostInvalid)
    );
    ensure!(
        host.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-:[]".contains(&b)),
        Message::plain(Text::HostCharacters)
    );
    if host.contains(':') {
        host.trim_matches(['[', ']'])
            .parse::<std::net::Ipv6Addr>()
            .context(Message::plain(Text::Ipv6Invalid))?;
    }
    Ok(())
}
pub fn validate_user(user: &str) -> Result<()> {
    ensure!(
        user.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
        Message::plain(Text::UserInvalid)
    );
    Ok(())
}
pub fn validate_target(target: &str) -> Result<()> {
    if let Some((user, host)) = target.split_once('@') {
        ensure!(!user.is_empty(), Message::plain(Text::UserEmpty));
        validate_user(user)?;
        validate_host(host)
    } else {
        validate_host(target)
    }
}
pub fn validate_jump(target: &str) -> Result<()> {
    let host = target.rsplit_once('@').map_or(target, |(_, host)| host);
    if let Some((name, port)) = host.rsplit_once(':')
        && (!name.contains(':') || name.ends_with(']'))
    {
        let port: u16 = port
            .parse()
            .context(Message::plain(Text::JumpPortInvalid))?;
        ensure!(port > 0, Message::plain(Text::JumpPortZero));
        let base = target.rsplit_once(':').expect("validated port suffix").0;
        return validate_target(base);
    }
    validate_target(target)
}
fn identity(path: &str) -> String {
    if let Some(suffix) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return std::path::PathBuf::from(home)
            .join(suffix)
            .to_string_lossy()
            .into_owned();
    }
    path.into()
}
fn profile_args(profile: &Profile) -> Vec<String> {
    let mut args = Vec::new();
    if !profile.user.is_empty() {
        args.extend(["-l".into(), profile.user.clone()]);
    }
    if let Some(port) = profile.port {
        args.extend(["-p".into(), port.to_string()]);
    }
    if !profile.identity.is_empty() {
        args.extend(["-i".into(), identity(&profile.identity)]);
    }
    if !profile.jump.is_empty() {
        args.extend(["-J".into(), profile.jump.clone()]);
    }
    args
}

pub struct EffectiveTarget {
    pub user: String,
    pub password_host: String,
    pub has_external_route: bool,
    pub preferred_authentications: String,
}

pub fn effective_target(profile: &Profile) -> Result<EffectiveTarget> {
    profile.validate()?;
    let output = Command::new("ssh")
        .arg("-G")
        .args(profile_args(profile))
        .arg("--")
        .arg(&profile.host)
        .output()
        .context(Message::plain(Text::SshMissing))?;
    ensure!(output.status.success(), Message::plain(Text::SshEffective));
    let values = String::from_utf8(output.stdout)?;
    let get = |key: &str| {
        values
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key} ")))
    };
    Ok(EffectiveTarget {
        user: get("user").context(Message::plain(Text::UserEmpty))?.into(),
        password_host: get("hostkeyalias")
            .filter(|value| *value != "none")
            .or_else(|| get("hostname"))
            .context(Message::plain(Text::SshHostMissing))?
            .into(),
        has_external_route: ["proxyjump", "proxycommand"]
            .iter()
            .any(|key| get(key).is_some_and(|value| value != "none")),
        preferred_authentications: get("preferredauthentications")
            .unwrap_or("publickey,keyboard-interactive,password")
            .into(),
    })
}
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''").replace('%', "%%"))
}

pub fn command(profile: &Profile, proxy: Option<&Proxy>) -> Result<CommandBuilder> {
    command_with_language(profile, proxy, Language::En)
}

pub fn command_with_language(
    profile: &Profile,
    proxy: Option<&Proxy>,
    language: Language,
) -> Result<CommandBuilder> {
    command_impl(profile, proxy, language, None)
}

pub fn command_with_authentication(
    profile: &Profile,
    proxy: Option<&Proxy>,
    language: Language,
    preferred: &str,
) -> Result<CommandBuilder> {
    command_impl(profile, proxy, language, Some(preferred))
}

fn command_impl(
    profile: &Profile,
    proxy: Option<&Proxy>,
    language: Language,
    preferred: Option<&str>,
) -> Result<CommandBuilder> {
    profile.validate()?;
    let mut args = vec![
        "-tt".into(),
        "-e".into(),
        "none".into(),
        "-o".into(),
        "StrictHostKeyChecking=ask".into(),
        "-o".into(),
        "ControlMaster=no".into(),
        "-o".into(),
        "ControlPath=none".into(),
        "-o".into(),
        "ControlPersist=no".into(),
        "-o".into(),
        "ConnectTimeout=15".into(),
        "-o".into(),
        "ServerAliveInterval=30".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
    ];
    if let Some(proxy) = proxy {
        proxy.validate()?;
        ensure!(
            profile.jump.is_empty(),
            Message::plain(Text::HttpJumpConflict)
        );
        let effective = Command::new("ssh")
            .arg("-G")
            .args(profile_args(profile))
            .arg("--")
            .arg(&profile.host)
            .output()
            .context(Message::plain(Text::SshMissing))?;
        ensure!(
            effective.status.success(),
            Message::plain(Text::SshEffective)
        );
        let values = String::from_utf8(effective.stdout)?;
        let get = |key: &str| {
            values
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{key} ")))
        };
        for name in ["proxyjump", "proxycommand"] {
            ensure!(
                get(name).is_none_or(|v| v == "none"),
                Message::arg(Text::SshRouteConflict, name)
            );
        }
        let host = get("hostname").context(Message::plain(Text::SshHostMissing))?;
        validate_host(host)?;
        let port: u16 = get("port")
            .context(Message::plain(Text::SshPortMissing))?
            .parse()?;
        let exe = std::env::current_exe()?;
        let exe = exe.to_str().context(Message::plain(Text::ExeUtf8))?;
        let helper = format!(
            "ProxyCommand={} __proxy-connect --proxy {} --proxy-user {} --host {} --port {} --language {}",
            shell_quote(exe),
            shell_quote(&proxy.url),
            shell_quote(&proxy.username),
            shell_quote(host),
            port,
            language.code()
        );
        args.extend(["-o".into(), helper]);
    }
    if let Some(preferred) = preferred {
        args.extend(["-o".into(), format!("PreferredAuthentications={preferred}")]);
    }
    args.extend(profile_args(profile));
    args.extend(["--".into(), profile.host.clone()]);
    let mut cmd = CommandBuilder::new("ssh");
    cmd.args(args);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_injection_is_rejected() {
        for host in [
            "-oProxyCommand=evil",
            "a;touch /tmp/x",
            "a'",
            "a\n",
            "a$(id)",
            "a%b",
            "user@a@b",
        ] {
            assert!(validate_target(host).is_err());
        }
        for host in ["user@example.org", "::1", "[2001:db8::1]", "board-1"] {
            validate_target(host).unwrap();
        }
        assert_eq!(shell_quote("/tmp/a'b%"), "'/tmp/a'\\''b%%'");
        for jump in ["user@bastion:2222", "bastion:022", "user@[::1]:2222", "::1"] {
            validate_jump(jump).unwrap();
        }
        assert!(validate_jump("bastion:0").is_err());
    }
    #[test]
    fn proxy_and_jump_conflict_before_connect() {
        let mut profile = Profile::quick("example.org".into());
        profile.jump = "bastion".into();
        let proxy = Proxy {
            name: "p".into(),
            url: "http://127.0.0.1:7890".into(),
            username: String::new(),
        };
        assert!(command(&profile, Some(&proxy)).is_err());
    }
}
