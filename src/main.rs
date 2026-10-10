use anyhow::{Result, ensure};
use clap::Parser;
use ssh_tui::i18n::{Context, Diagnostic, Language, Message, Text};
use ssh_tui::{
    app::App,
    config::{Config, Profile, Proxy, Route},
    theme,
};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version,
    about = "Linux SSH terminal manager with tabs, themes and HTTP CONNECT proxies"
)]
struct Args {
    /// [user@]host or an alias from ~/.ssh/config
    target: Option<String>,
    /// UI language for this run (overrides configuration)
    #[arg(long, value_enum)]
    language: Option<Language>,
    #[arg(long, short = 'p', requires = "target")]
    port: Option<u16>,
    #[arg(long, short = 'i', requires = "target")]
    identity: Option<String>,
    #[arg(long)]
    theme: Option<String>,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long, conflicts_with = "no_proxy")]
    proxy: Option<String>,
    #[arg(long, requires = "proxy")]
    proxy_user: Option<String>,
    /// Disable the application proxy; still use OpenSSH configuration
    #[arg(long)]
    no_proxy: bool,
}

#[derive(Parser)]
struct Helper {
    #[arg(long, value_enum, default_value = "en")]
    language: Language,
    #[arg(long)]
    proxy: String,
    #[arg(long, default_value = "")]
    proxy_user: String,
    #[arg(long)]
    host: String,
    #[arg(long)]
    port: u16,
}

fn main() {
    let mut language = Language::En;
    if let Err(error) = run(&mut language) {
        eprintln!("ssh-tui: {}", Diagnostic::capture(&error).render(language));
        std::process::exit(1);
    }
}
fn run(language: &mut Language) -> Result<()> {
    if std::env::args().nth(1).as_deref() == Some("__proxy-connect") {
        let helper = Helper::parse_from(std::env::args().skip(1));
        *language = helper.language;
        return ssh_tui::proxy::run(
            &helper.proxy,
            &helper.proxy_user,
            &helper.host,
            helper.port,
            *language,
        );
    }
    let args = Args::parse();
    *language = args.language.unwrap_or_default();
    let path = args
        .config
        .map(Ok)
        .unwrap_or_else(ssh_tui::config::default_path)?;
    let cfg = Config::load(&path).with_context(|| Message::arg(Text::ReadPath, path.display()))?;
    *language = args.language.unwrap_or(cfg.language);
    let mut proxy_override = if args.no_proxy { Some(None) } else { None };
    if let Some(url) = args.proxy {
        let proxy = Proxy {
            name: "__command_line__".into(),
            url,
            username: args.proxy_user.unwrap_or_default(),
        };
        proxy.validate()?;
        proxy_override = Some(Some(proxy));
    }
    let themes = theme::load(
        &path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("themes"),
    )?;
    let theme_name = args.theme.as_deref().unwrap_or(&cfg.theme);
    let selected_theme = themes
        .iter()
        .position(|t| t.name == theme_name)
        .context(Message::plain(Text::ThemeMissing))?;
    let initial = args.target.map(|host| {
        let mut p = Profile::quick(host);
        p.port = args.port;
        p.identity = args.identity.unwrap_or_default();
        if args.no_proxy {
            p.route = Route::Direct;
        }
        p
    });
    if let Some(profile) = &initial {
        profile.validate()?;
    }
    use std::io::IsTerminal;
    ensure!(
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        Message::plain(Text::NeedTerminal)
    );
    App::new(cfg, path, themes, selected_theme)
        .with_language(*language)
        .with_proxy_override(proxy_override)
        .run(initial)
}
