use crate::i18n::{Context, Diagnostic, Language, Message, Text};
use crate::{
    config::{Config, Profile, Proxy, Route, parse_route, route_text},
    input,
    session::Session,
    terminal::ThemedScreen,
    theme::Theme,
    ui::{self, ListRow, Panel},
};
use anyhow::Result;
use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tui_term::widget::{Cursor, PseudoTerminal};

#[derive(Clone)]
enum FormKind {
    Profile { edit: Option<usize>, quick: bool },
    Proxy { edit: Option<usize> },
}
#[derive(Clone)]
struct Form {
    kind: FormKind,
    fields: Vec<String>,
    cursors: Vec<usize>,
    selected: usize,
    password: zeroize::Zeroizing<String>,
    saved_credential: bool,
    remember_password: bool,
}
impl Form {
    fn profile(profile: Profile, edit: Option<usize>, quick: bool) -> Self {
        let saved_credential = profile.credential_id.is_some();
        let mut fields = vec![
            profile.name,
            profile.host,
            profile.user,
            profile.port.map(|p| p.to_string()).unwrap_or_default(),
            profile.identity,
            profile.jump,
            route_text(&profile.route),
        ];
        if !quick {
            fields.extend([String::new(), String::new()]);
        }
        let mut form = Self::new(FormKind::Profile { edit, quick }, fields);
        form.saved_credential = saved_credential;
        form.remember_password = saved_credential;
        form
    }
    fn proxy(proxy: Proxy, edit: Option<usize>) -> Self {
        Self::new(
            FormKind::Proxy { edit },
            vec![proxy.name, proxy.url, proxy.username],
        )
    }
    fn new(kind: FormKind, fields: Vec<String>) -> Self {
        let cursors = fields.iter().map(String::len).collect();
        Self {
            kind,
            fields,
            cursors,
            selected: 0,
            password: zeroize::Zeroizing::new(String::new()),
            saved_credential: false,
            remember_password: false,
        }
    }
    fn edit(&mut self, key: KeyEvent) {
        let i = self.selected;
        if i == 8 && matches!(self.kind, FormKind::Profile { .. }) {
            match key.code {
                KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right => {
                    self.remember_password = !self.remember_password;
                }
                KeyCode::Tab | KeyCode::Down => self.selected = 0,
                KeyCode::BackTab | KeyCode::Up => self.selected = 7,
                _ => {}
            }
            return;
        }
        let text = if i == 7 {
            &mut *self.password
        } else {
            &mut self.fields[i]
        };
        let cursor = &mut self.cursors[i];
        match key.code {
            KeyCode::Tab | KeyCode::Down => self.selected = (i + 1) % self.fields.len(),
            KeyCode::BackTab | KeyCode::Up => {
                self.selected = (i + self.fields.len() - 1) % self.fields.len()
            }
            KeyCode::Home => *cursor = 0,
            KeyCode::End => *cursor = text.len(),
            KeyCode::Left | KeyCode::Backspace if *cursor > 0 => {
                let prev = text[..*cursor].char_indices().last().map_or(0, |(n, _)| n);
                if key.code == KeyCode::Backspace {
                    text.replace_range(prev..*cursor, "");
                }
                *cursor = prev;
            }
            KeyCode::Right | KeyCode::Delete if *cursor < text.len() => {
                let next = *cursor + text[*cursor..].chars().next().unwrap().len_utf8();
                if key.code == KeyCode::Delete {
                    text.replace_range(*cursor..next, "");
                } else {
                    *cursor = next;
                }
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                text.clear();
                *cursor = 0;
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                text.insert(*cursor, c);
                *cursor += c.len_utf8();
            }
            _ => {}
        }
    }
    fn paste(&mut self, text: &str) -> Result<()> {
        if self.selected == 7 {
            anyhow::ensure!(
                !text.contains(['\r', '\n', '\0']),
                Message::plain(Text::SshPasswordInvalid)
            );
            self.password.insert_str(self.cursors[7], text);
            self.cursors[7] += text.len();
            return Ok(());
        }
        if self.selected == 8 {
            return Ok(());
        }
        let text = text.replace(['\r', '\n', '\0'], "");
        let i = self.selected;
        self.fields[i].insert_str(self.cursors[i], &text);
        self.cursors[i] += text.len();
        Ok(())
    }
    fn display_field(&self, i: usize, language: Language) -> (String, usize) {
        if i == 7 {
            if self.password.is_empty() && self.saved_credential && self.remember_password {
                return (language.text(Text::PasswordSaved).into(), 0);
            }
            let value = "*".repeat(self.password.chars().count());
            let cursor = self.password[..self.cursors[i]].chars().count();
            return (value, cursor);
        }
        if i == 8 {
            return (
                language
                    .text(if self.remember_password {
                        Text::On
                    } else {
                        Text::Off
                    })
                    .into(),
                0,
            );
        }
        (self.fields[i].clone(), self.cursors[i])
    }
}
#[derive(Clone)]
enum Confirm {
    Close(usize),
    Quit,
    DeleteProfile(usize),
    DeleteProxy(usize),
}
#[derive(Clone)]
enum View {
    Manager,
    Terminal,
    Proxies,
    Form(Form),
    Themes {
        previous: usize,
    },
    Languages {
        original: Language,
        previous: Box<View>,
    },
    Help {
        scroll: usize,
        previous: Box<View>,
    },
    Details {
        text: String,
        scroll: usize,
        previous: Box<View>,
    },
    Scroll,
    Confirm(Confirm),
}

struct Notice {
    text: Diagnostic,
    error: bool,
    expires: Option<Instant>,
}

pub struct App {
    config: Config,
    language: Language,
    proxy_override: Option<Option<Proxy>>,
    path: PathBuf,
    themes: Vec<Theme>,
    theme: usize,
    sessions: Vec<Session>,
    active: usize,
    view: View,
    prefix: bool,
    selected: usize,
    proxy_selected: usize,
    filter: String,
    searching: bool,
    notice: Option<Notice>,
    rows: u16,
    cols: u16,
    quit: bool,
    credentials: Arc<dyn crate::credentials::CredentialStore>,
}
impl App {
    pub fn new(config: Config, path: PathBuf, themes: Vec<Theme>, theme: usize) -> Self {
        Self {
            language: config.language,
            config,
            proxy_override: None,
            path,
            themes,
            theme,
            sessions: Vec::new(),
            active: 0,
            view: View::Manager,
            prefix: false,
            selected: 0,
            proxy_selected: 0,
            filter: String::new(),
            searching: false,
            notice: None,
            rows: 24,
            cols: 80,
            quit: false,
            credentials: Arc::new(crate::credentials::SystemStore),
        }
    }
    pub fn with_proxy_override(mut self, proxy: Option<Option<Proxy>>) -> Self {
        self.proxy_override = proxy;
        self
    }
    pub fn with_language(mut self, language: Language) -> Self {
        self.language = language;
        self
    }
    fn t(&self, key: Text) -> &'static str {
        self.language.text(key)
    }
    fn start_languages(&mut self) {
        self.view = View::Languages {
            original: self.language,
            previous: Box::new(self.view.clone()),
        };
    }
    fn resolve_proxy(&self, profile: &Profile) -> Result<Option<Proxy>> {
        if profile.route == Route::Inherit
            && let Some(proxy) = &self.proxy_override
        {
            return Ok(proxy.clone());
        }
        self.config.route(profile)
    }
    pub fn run(mut self, initial: Option<Profile>) -> Result<()> {
        let stopped = Arc::new(AtomicBool::new(false));
        let signal = stopped.clone();
        ctrlc::set_handler(move || signal.store(true, Ordering::Relaxed))?;
        // Ratatui installs a panic hook that restores terminal state.
        let mut terminal = ratatui::try_init()?;
        struct Restore;
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = execute!(std::io::stdout(), DisableBracketedPaste);
                ratatui::restore();
            }
        }
        let _restore = Restore;
        execute!(std::io::stdout(), EnableBracketedPaste)?;
        let size = terminal.size()?;
        self.dimensions(size.width, size.height)?;
        if let Some(profile) = initial
            && let Err(e) = self.connect(profile)
        {
            self.error(Diagnostic::capture(&e));
        }
        let result = self.event_loop(&mut terminal, &stopped);
        for session in &mut self.sessions {
            session.close();
        }
        result
    }
    fn event_loop(&mut self, terminal: &mut DefaultTerminal, stopped: &AtomicBool) -> Result<()> {
        let mut last = Instant::now() - Duration::from_secs(1);
        let mut dirty = true;
        while !self.quit && !stopped.load(Ordering::Relaxed) {
            if self
                .notice
                .as_ref()
                .is_some_and(|n| n.expires.is_some_and(|at| Instant::now() >= at))
            {
                self.notice = None;
                dirty = true;
            }
            for (i, s) in self.sessions.iter_mut().enumerate() {
                let visible = i == self.active && matches!(self.view, View::Terminal);
                match s
                    .poll(i == self.active)
                    .and_then(|changed| s.poll_image(visible).map(|image| changed || image))
                {
                    Ok(changed) => dirty |= changed,
                    Err(e) => {
                        s.status = Diagnostic::capture(&e);
                        dirty = true;
                    }
                }
            }
            if dirty && last.elapsed() >= Duration::from_millis(33) {
                terminal.draw(|f| self.draw(f))?;
                last = Instant::now();
                dirty = false;
            }
            if event::poll(Duration::from_millis(10))? {
                let result = match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => self.on_key(key),
                    Event::Paste(text) => self.on_paste(&text),
                    Event::Resize(w, h) => self.dimensions(w, h),
                    _ => Ok(()),
                };
                if let Err(e) = result {
                    self.error(Diagnostic::capture(&e));
                }
                dirty = true;
            }
        }
        Ok(())
    }
    fn dimensions(&mut self, w: u16, h: u16) -> Result<()> {
        self.cols = w.max(1);
        self.rows = h.saturating_sub(2).max(1);
        for s in &mut self.sessions {
            s.resize(self.rows, self.cols)?;
        }
        Ok(())
    }
    fn connect(&mut self, profile: Profile) -> Result<()> {
        self.connect_mode(profile, false)
    }
    fn connect_mode(&mut self, profile: Profile, manual: bool) -> Result<()> {
        let proxy = self.resolve_proxy(&profile)?;
        let s = Session::open_with_credentials(
            profile,
            proxy,
            self.rows,
            self.cols,
            self.config.scrollback,
            self.language,
            if manual {
                None
            } else {
                Some(self.credentials.clone())
            },
        )?;
        self.sessions.push(s);
        self.active = self.sessions.len() - 1;
        self.view = View::Terminal;
        self.notice = None;
        Ok(())
    }
    fn persist(&mut self, next: Config) -> Result<()> {
        next.save(&self.path)?;
        self.config = next;
        self.notice = Some(Notice {
            text: Message::plain(Text::Saved).into(),
            error: false,
            expires: Some(Instant::now() + Duration::from_secs(3)),
        });
        self.cleanup_credentials();
        Ok(())
    }
    fn cleanup_credentials(&mut self) {
        if self.config.pending_credential_cleanup.is_empty() {
            return;
        }
        let mut next = self.config.clone();
        next.pending_credential_cleanup
            .retain(|id| self.credentials.delete(id).is_err());
        if next.pending_credential_cleanup != self.config.pending_credential_cleanup
            && next.save(&self.path).is_ok()
        {
            self.config = next;
        }
        if !self.config.pending_credential_cleanup.is_empty() {
            self.error(Message::plain(Text::CredentialCleanup));
        }
    }
    fn error(&mut self, text: impl Into<Diagnostic>) {
        self.notice = Some(Notice {
            text: text.into(),
            error: true,
            expires: None,
        });
    }
    fn start_help(&mut self) {
        self.view = View::Help {
            scroll: 0,
            previous: Box::new(self.view.clone()),
        };
    }
    fn start_details(&mut self) {
        let text = self
            .notice
            .as_ref()
            .map(|n| n.text.render(self.language))
            .or_else(|| {
                self.sessions
                    .get(self.active)
                    .map(|s| format!("{}\n{}", s.route_label, s.status.render(self.language)))
            })
            .unwrap_or_else(|| self.t(Text::NoDetails).into());
        self.view = View::Details {
            text,
            scroll: 0,
            previous: Box::new(self.view.clone()),
        };
    }
    fn matches(&self) -> Vec<usize> {
        let filter = self.filter.to_lowercase();
        self.config
            .connections
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                format!("{} {} {}", p.name, p.host, p.user)
                    .to_lowercase()
                    .contains(&filter)
            })
            .map(|(i, _)| i)
            .collect()
    }
    fn selected_profile(&self) -> Option<usize> {
        self.matches().get(self.selected).copied()
    }
    fn return_terminal(&mut self) {
        self.view = if self.sessions.is_empty() {
            View::Manager
        } else {
            View::Terminal
        };
    }
    fn request_quit(&mut self) {
        if self.sessions.iter().any(|s| !s.exited) {
            self.view = View::Confirm(Confirm::Quit);
        } else {
            self.quit = true;
        }
    }
    fn start_themes(&mut self) {
        self.view = View::Themes {
            previous: self.theme,
        };
    }
    fn on_paste(&mut self, text: &str) -> Result<()> {
        match &mut self.view {
            View::Form(form) => form.paste(text)?,
            View::Terminal => {
                if let Some(s) = self.sessions.get_mut(self.active) {
                    if s.image.busy() {
                        return Ok(());
                    }
                    let bytes = input::paste(text, s.parser.screen().bracketed_paste());
                    s.write(&bytes)?;
                }
            }
            View::Manager if self.searching => {
                self.filter.push_str(&text.replace(['\r', '\n'], ""))
            }
            _ => {}
        }
        Ok(())
    }
    fn on_key(&mut self, key: KeyEvent) -> Result<()> {
        if !matches!(
            self.view,
            View::Terminal | View::Help { .. } | View::Details { .. }
        ) && key.code == KeyCode::F(2)
        {
            self.start_details();
            return Ok(());
        }
        if !input::is_prefix(key)
            && key.code != KeyCode::Char('!')
            && !matches!(self.view, View::Details { .. })
            && self.notice.as_ref().is_some_and(|n| n.error)
        {
            self.notice = None;
        }
        if matches!(self.view, View::Terminal) {
            if self.prefix {
                self.prefix = false;
                return self.prefix_key(key);
            }
            if input::is_prefix(key) {
                self.prefix = true;
                return Ok(());
            }
            if let Some(s) = self.sessions.get_mut(self.active) {
                if s.exited && key.code == KeyCode::Char('r') {
                    return self.reconnect_mode(false);
                }
                if s.exited && key.code == KeyCode::Char('m') {
                    return self.reconnect_mode(true);
                }
                if s.image.busy() {
                    if key.code == KeyCode::Esc {
                        s.cancel_image();
                    }
                    return Ok(());
                }
                if s.image.enabled
                    && key.code == KeyCode::Char('v')
                    && key.modifiers == KeyModifiers::CONTROL
                {
                    return s.start_image();
                }
                let screen = s.parser.screen();
                let bytes = input::encode(
                    key,
                    screen.application_cursor(),
                    screen.application_keypad(),
                );
                s.write(&bytes)?;
            }
            return Ok(());
        }
        let view = std::mem::replace(&mut self.view, View::Terminal);
        self.view = view.clone();
        match view {
            View::Manager => self.manager_key(key)?,
            View::Proxies => self.proxies_key(key)?,
            View::Form(mut form) => {
                if key.code == KeyCode::Esc {
                    self.view = match form.kind {
                        FormKind::Profile { .. } => View::Manager,
                        _ => View::Proxies,
                    };
                } else if key.code == KeyCode::Enter {
                    self.submit(&form)?;
                } else {
                    form.edit(key);
                    self.view = View::Form(form);
                }
            }
            View::Themes { previous } => match key.code {
                KeyCode::Esc => {
                    self.theme = previous;
                    self.return_terminal();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.theme = (self.theme + self.themes.len() - 1) % self.themes.len()
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.theme = (self.theme + 1) % self.themes.len()
                }
                KeyCode::PageUp => {
                    self.theme = self.theme.saturating_sub(self.theme_page_rows());
                }
                KeyCode::PageDown => {
                    self.theme = (self.theme + self.theme_page_rows()).min(self.themes.len() - 1);
                }
                KeyCode::Home => self.theme = 0,
                KeyCode::End => self.theme = self.themes.len() - 1,
                KeyCode::Enter => {
                    let mut next = self.config.clone();
                    next.theme = self.themes[self.theme].name.clone();
                    self.persist(next)?;
                    self.return_terminal();
                }
                _ => {}
            },
            View::Languages { original, previous } => match key.code {
                KeyCode::Esc => {
                    self.language = original;
                    self.view = *previous;
                }
                KeyCode::Up | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('k') => {
                    self.language = if self.language == Language::En {
                        Language::Zh
                    } else {
                        Language::En
                    };
                }
                KeyCode::Enter => {
                    let mut next = self.config.clone();
                    next.language = self.language;
                    self.persist(next)?;
                    self.view = *previous;
                }
                _ => {}
            },
            View::Help { scroll, previous } => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Enter) {
                    self.view = *previous;
                } else {
                    self.view = View::Help {
                        scroll: self.text_scroll(scroll, key, self.t(Text::HelpText)),
                        previous,
                    };
                }
            }
            View::Details {
                text,
                scroll,
                previous,
            } => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                    self.view = *previous;
                } else {
                    self.view = View::Details {
                        scroll: self.text_scroll(scroll, key, &text),
                        text,
                        previous,
                    };
                }
            }
            View::Scroll => {
                if let Some(s) = self.sessions.get_mut(self.active) {
                    let old = s.parser.screen().scrollback();
                    let n = match key.code {
                        KeyCode::Up | KeyCode::Char('k') => old.saturating_add(1),
                        KeyCode::Down | KeyCode::Char('j') => old.saturating_sub(1),
                        KeyCode::PageUp => old.saturating_add(self.rows as usize),
                        KeyCode::PageDown => old.saturating_sub(self.rows as usize),
                        KeyCode::Home => self.config.scrollback,
                        KeyCode::End => 0,
                        KeyCode::Esc => {
                            self.view = View::Terminal;
                            0
                        }
                        _ => old,
                    };
                    s.parser.screen_mut().set_scrollback(n);
                } else {
                    self.view = View::Manager;
                }
            }
            View::Confirm(action) => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.confirm(action)?,
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.view = match action {
                        Confirm::DeleteProfile(_) => View::Manager,
                        Confirm::DeleteProxy(_) => View::Proxies,
                        _ => {
                            if self.sessions.is_empty() {
                                View::Manager
                            } else {
                                View::Terminal
                            }
                        }
                    }
                }
                _ => {}
            },
            View::Terminal => {}
        }
        Ok(())
    }
    fn prefix_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {}
            _ if input::is_prefix(key) => {
                if let Some(s) = self.sessions.get_mut(self.active)
                    && !s.image.busy()
                {
                    s.write(&[29])?;
                }
            }
            KeyCode::Char('v') => {
                if let Some(s) = self.sessions.get_mut(self.active) {
                    s.toggle_images()?;
                }
            }
            KeyCode::Char('c') => self.view = View::Manager,
            KeyCode::Char('n') if !self.sessions.is_empty() => {
                self.active = (self.active + 1) % self.sessions.len()
            }
            KeyCode::Char('p') if !self.sessions.is_empty() => {
                self.active = (self.active + self.sessions.len() - 1) % self.sessions.len()
            }
            KeyCode::Char(n @ '1'..='9') => {
                let i = n as usize - '1' as usize;
                if i < self.sessions.len() {
                    self.active = i;
                }
            }
            KeyCode::Char('t') => self.start_themes(),
            KeyCode::Char('l') => self.start_languages(),
            KeyCode::Char('?') => self.start_help(),
            KeyCode::Char('!') => self.start_details(),
            KeyCode::Char('[') => self.view = View::Scroll,
            KeyCode::Char('x') if !self.sessions.is_empty() => {
                if self.sessions[self.active].exited {
                    self.confirm(Confirm::Close(self.active))?;
                } else {
                    self.view = View::Confirm(Confirm::Close(self.active));
                }
            }
            KeyCode::Char('q') => self.request_quit(),
            _ => {
                if let Some(s) = self.sessions.get_mut(self.active) {
                    let screen = s.parser.screen();
                    if s.image.busy() {
                        return Ok(());
                    }
                    let mut bytes = vec![29];
                    bytes.extend(input::encode(
                        key,
                        screen.application_cursor(),
                        screen.application_keypad(),
                    ));
                    s.write(&bytes)?;
                }
            }
        }
        Ok(())
    }
    fn manager_key(&mut self, key: KeyEvent) -> Result<()> {
        if self.searching {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => self.searching = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => self.filter.push(c),
                _ => {}
            }
            self.selected = 0;
            return Ok(());
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.matches().len().saturating_sub(1))
            }
            KeyCode::Char('/') => self.searching = true,
            KeyCode::Char('a') => {
                self.view = View::Form(Form::profile(Profile::quick(String::new()), None, false))
            }
            KeyCode::Char('c') => {
                self.view = View::Form(Form::profile(Profile::quick(String::new()), None, true))
            }
            KeyCode::Char('e') => {
                if let Some(i) = self.selected_profile() {
                    self.view = View::Form(Form::profile(
                        self.config.connections[i].clone(),
                        Some(i),
                        false,
                    ));
                }
            }
            KeyCode::Char('d') => {
                if let Some(i) = self.selected_profile() {
                    self.view = View::Confirm(Confirm::DeleteProfile(i));
                }
            }
            KeyCode::Enter => {
                if let Some(i) = self.selected_profile() {
                    self.connect(self.config.connections[i].clone())?;
                }
            }
            KeyCode::Char('m') => {
                if let Some(i) = self.selected_profile() {
                    self.connect_mode(self.config.connections[i].clone(), true)?;
                }
            }
            KeyCode::Char('p') => self.view = View::Proxies,
            KeyCode::Char('t') => self.start_themes(),
            KeyCode::Char('l') => self.start_languages(),
            KeyCode::Char('?') => self.start_help(),
            KeyCode::Char('!') => self.start_details(),
            KeyCode::Char('q') => self.request_quit(),
            KeyCode::Esc | KeyCode::Tab => self.return_terminal(),
            _ => {}
        }
        Ok(())
    }
    fn proxies_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.view = View::Manager,
            KeyCode::Up | KeyCode::Char('k') => {
                self.proxy_selected = self.proxy_selected.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.proxy_selected =
                    (self.proxy_selected + 1).min(self.config.proxies.len().saturating_sub(1))
            }
            KeyCode::Char('a') => {
                self.view = View::Form(Form::proxy(
                    Proxy {
                        name: String::new(),
                        url: "http://127.0.0.1:7890".into(),
                        username: String::new(),
                    },
                    None,
                ))
            }
            KeyCode::Char('e') => {
                if let Some(p) = self.config.proxies.get(self.proxy_selected) {
                    self.view = View::Form(Form::proxy(p.clone(), Some(self.proxy_selected)));
                }
            }
            KeyCode::Char('d') => {
                if self.proxy_selected < self.config.proxies.len() {
                    self.view = View::Confirm(Confirm::DeleteProxy(self.proxy_selected));
                }
            }
            KeyCode::Enter => {
                if let Some(p) = self.config.proxies.get(self.proxy_selected) {
                    let mut next = self.config.clone();
                    next.default_proxy = Some(p.name.clone());
                    self.persist(next)?;
                }
            }
            KeyCode::Char('0') => {
                let mut next = self.config.clone();
                next.default_proxy = None;
                self.persist(next)?;
            }
            KeyCode::Char('l') => self.start_languages(),
            KeyCode::Char('?') => self.start_help(),
            KeyCode::Char('!') => self.start_details(),
            _ => {}
        }
        Ok(())
    }
    fn submit(&mut self, form: &Form) -> Result<()> {
        let fields = &form.fields;
        match form.kind {
            FormKind::Profile { edit, quick } => {
                let mut p = Profile {
                    name: fields[0].trim().into(),
                    host: fields[1].trim().into(),
                    user: fields[2].trim().into(),
                    port: if fields[3].is_empty() {
                        None
                    } else {
                        Some(
                            fields[3]
                                .parse()
                                .context(Message::plain(Text::PortNumber))?,
                        )
                    },
                    identity: fields[4].clone(),
                    jump: fields[5].trim().into(),
                    route: parse_route(&fields[6])?,
                    credential_id: edit
                        .and_then(|i| self.config.connections[i].credential_id.clone()),
                };
                p.validate()?;
                self.config.route(&p)?;
                if quick {
                    self.connect(p)?;
                } else {
                    let mut next = self.config.clone();
                    let old_id = p.credential_id.clone();
                    if form.remember_password {
                        if form.password.is_empty() {
                            anyhow::ensure!(
                                old_id.is_some(),
                                Message::plain(Text::PasswordRequired)
                            );
                            if let Some(i) = edit {
                                let old = &self.config.connections[i];
                                anyhow::ensure!(
                                    old.host == p.host && old.user == p.user && old.port == p.port,
                                    Message::plain(Text::CredentialTargetChanged)
                                );
                            }
                        } else {
                            crate::credentials::validate_password(&form.password)?;
                            p.credential_id = Some(crate::credentials::new_id());
                        }
                    } else {
                        p.credential_id = None;
                    }
                    let new_id = p.credential_id.clone();
                    if old_id != new_id
                        && let Some(id) = old_id
                    {
                        next.pending_credential_cleanup.push(id);
                    }
                    if let Some(i) = edit {
                        next.connections[i] = p;
                    } else {
                        next.connections.push(p);
                    }
                    next.validate()?;
                    let created =
                        new_id.filter(|_| !form.password.is_empty() && form.remember_password);
                    if let Some(id) = &created {
                        self.credentials.set(id, &form.password)?;
                    }
                    if let Err(error) = self.persist(next) {
                        if let Some(id) = created {
                            // rename may have succeeded even if directory fsync failed.
                            let committed = Config::load(&self.path).ok().is_some_and(|cfg| {
                                cfg.connections
                                    .iter()
                                    .any(|p| p.credential_id.as_ref() == Some(&id))
                            });
                            if !committed && self.credentials.delete(&id).is_err() {
                                self.config.pending_credential_cleanup.push(id);
                            }
                        }
                        return Err(error);
                    }
                    self.view = View::Manager;
                    self.filter.clear();
                    self.selected = 0;
                }
            }
            FormKind::Proxy { edit } => {
                let p = Proxy {
                    name: fields[0].trim().into(),
                    url: fields[1].trim().into(),
                    username: fields[2].clone(),
                };
                p.validate()?;
                let mut next = self.config.clone();
                if let Some(i) = edit {
                    let old = next.proxies[i].name.clone();
                    if next.default_proxy.as_ref() == Some(&old) {
                        next.default_proxy = Some(p.name.clone());
                    }
                    for profile in &mut next.connections {
                        if profile.route == Route::Http(old.clone()) {
                            profile.route = Route::Http(p.name.clone());
                        }
                    }
                    next.proxies[i] = p;
                } else {
                    next.proxies.push(p);
                }
                self.persist(next)?;
                self.view = View::Proxies;
            }
        }
        Ok(())
    }
    fn confirm(&mut self, action: Confirm) -> Result<()> {
        match action {
            Confirm::Quit => self.quit = true,
            Confirm::Close(i) => {
                self.sessions.remove(i);
                self.active = self.active.min(self.sessions.len().saturating_sub(1));
                self.return_terminal();
            }
            Confirm::DeleteProfile(i) => {
                let mut next = self.config.clone();
                if let Some(id) = next.connections.remove(i).credential_id {
                    next.pending_credential_cleanup.push(id);
                }
                self.persist(next)?;
                self.selected = self.selected.min(self.matches().len().saturating_sub(1));
                self.view = View::Manager;
            }
            Confirm::DeleteProxy(i) => {
                let mut next = self.config.clone();
                let name = next.proxies[i].name.clone();
                anyhow::ensure!(
                    !next
                        .connections
                        .iter()
                        .any(|p| p.route == Route::Http(name.clone())),
                    Message::plain(Text::ProxyReferenced)
                );
                next.proxies.remove(i);
                if next.default_proxy.as_ref() == Some(&name) {
                    next.default_proxy = None;
                }
                self.persist(next)?;
                self.proxy_selected = self
                    .proxy_selected
                    .min(self.config.proxies.len().saturating_sub(1));
                self.view = View::Proxies;
            }
        }
        Ok(())
    }
    fn reconnect_mode(&mut self, manual: bool) -> Result<()> {
        let original = &self.sessions[self.active].profile;
        let profile = self
            .config
            .connections
            .iter()
            .find(|p| p.name == original.name)
            .unwrap_or(original)
            .clone();
        let proxy = self.resolve_proxy(&profile)?;
        self.sessions[self.active] = Session::open_with_credentials(
            profile,
            proxy,
            self.rows,
            self.cols,
            self.config.scrollback,
            self.language,
            if manual {
                None
            } else {
                Some(self.credentials.clone())
            },
        )?;
        Ok(())
    }
    fn draw(&self, f: &mut Frame) {
        let t = &self.themes[self.theme];
        let colors = t.ui();
        let regions = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area());
        f.render_widget(Block::default().style(colors.base()), f.area());
        let tabs: Vec<_> = self
            .sessions
            .iter()
            .map(|s| ui::Tab {
                name: s.profile.name.clone(),
                exited: s.exited,
                unread: s.unread,
            })
            .collect();
        ui::tabs(f, regions[0], colors, &tabs, self.active);
        if let Some(s) = self.sessions.get(self.active) {
            let screen =
                ThemedScreen::new(s.parser.screen(), t, matches!(self.view, View::Terminal));
            f.render_widget(
                PseudoTerminal::new(&screen)
                    .style(t.style())
                    .cursor(Cursor::default().symbol(" ")),
                regions[1],
            );
        }
        match &self.view {
            View::Manager => self.draw_manager(f, regions[1]),
            View::Proxies => self.draw_proxies(f, regions[1]),
            View::Form(form) => self.draw_form(f, regions[1], form),
            View::Themes { .. } => self.draw_themes(f, regions[1]),
            View::Languages { .. } => self.draw_languages(f, regions[1]),
            View::Help { scroll, .. } => self.draw_text(
                f,
                regions[1],
                self.t(Text::HelpTitle),
                self.t(Text::HelpText),
                *scroll,
                false,
            ),
            View::Details { text, scroll, .. } => self.draw_text(
                f,
                regions[1],
                self.t(Text::DetailsTitle),
                text,
                *scroll,
                false,
            ),
            View::Confirm(action) => {
                let text = match action {
                    Confirm::Quit => self.t(Text::ConfirmQuit),
                    Confirm::Close(_) => self.t(Text::ConfirmClose),
                    Confirm::DeleteProfile(_) => self.t(Text::ConfirmDeleteProfile),
                    Confirm::DeleteProxy(_) => self.t(Text::ConfirmDeleteProxy),
                };
                self.draw_text(f, regions[1], self.t(Text::Confirm), text, 0, true);
            }
            _ => {}
        }
        self.draw_status(f, regions[2]);
    }
    fn draw_status(&self, f: &mut Frame, area: Rect) {
        let t = &self.themes[self.theme];
        let mut colors = t.ui();
        colors.surface = colors.background;
        colors.foreground = crate::theme::readable(colors.foreground, colors.background);
        colors.muted = crate::theme::readable(colors.muted, colors.background);
        let actions: &[(&str, &str)] = match &self.view {
            View::Terminal if self.prefix => &[
                ("Ctrl+]", self.t(Text::Prefix)),
                ("?", self.t(Text::Help)),
                ("c", self.t(Text::Connect)),
                ("n/p", self.t(Text::Tabs)),
                ("t", self.t(Text::Theme)),
                ("l", self.t(Text::Language)),
                ("v", self.t(Text::ImageToggle)),
                ("!", self.t(Text::Details)),
            ],
            View::Terminal => &[
                ("Ctrl+] ?", self.t(Text::Help)),
                (
                    "Ctrl+] v",
                    self.t(
                        if self
                            .sessions
                            .get(self.active)
                            .is_some_and(|s| s.image.enabled)
                        {
                            Text::ImageOn
                        } else {
                            Text::ImageOff
                        },
                    ),
                ),
            ],
            View::Scroll => &[
                ("Esc", self.t(Text::Back)),
                ("↑↓", self.t(Text::Scrollback)),
                ("PgUp/PgDn", self.t(Text::Page)),
            ],
            View::Manager => &[
                ("p", self.t(Text::Proxy)),
                ("t", self.t(Text::Theme)),
                ("l", self.t(Text::Language)),
                ("c", self.t(Text::Quick)),
                ("?", self.t(Text::Help)),
            ],
            View::Proxies => &[
                ("Enter", self.t(Text::Default)),
                ("0", self.t(Text::DisableDefault)),
                ("l", self.t(Text::Language)),
                ("?", self.t(Text::Help)),
            ],
            View::Form(_) => &[
                ("Tab", self.t(Text::Change)),
                ("Ctrl+U", self.t(Text::Clear)),
                ("F2", self.t(Text::Details)),
            ],
            View::Themes { .. } => &[
                ("↑↓", self.t(Text::Preview)),
                ("Enter", self.t(Text::Save)),
                ("Esc", self.t(Text::Restore)),
                ("PgUp/PgDn", self.t(Text::Page)),
                ("Home/End", self.t(Text::FirstLast)),
            ],
            View::Languages { .. } => &[
                ("↑↓", self.t(Text::Preview)),
                ("Enter", self.t(Text::Save)),
                ("Esc", self.t(Text::Restore)),
            ],
            View::Help { .. } | View::Details { .. } => &[
                ("↑↓", self.t(Text::Scroll)),
                ("PgUp/PgDn", self.t(Text::Page)),
            ],
            View::Confirm(_) => &[
                ("y", self.t(Text::Confirm)),
                ("n/Esc", self.t(Text::Cancel)),
            ],
        };
        if matches!(self.view, View::Terminal)
            && !self.prefix
            && let Some(session) = self.sessions.get(self.active)
            && session.image.busy()
        {
            f.render_widget(
                Paragraph::new(self.t(Text::ImageBusy)).style(colors.base()),
                area,
            );
            return;
        }
        // Notices take priority; the ambient context is hidden until they clear.
        if let Some(notice) = &self.notice {
            let suffix = if notice.error {
                if matches!(self.view, View::Terminal) {
                    self.t(Text::PrefixDetails)
                } else {
                    self.t(Text::LocalDetails)
                }
            } else {
                ""
            };
            let room = usize::from(area.width).saturating_sub(ui::width(suffix) + 1);
            let text = if room > 0 {
                format!(
                    " {}{suffix}",
                    ui::truncate(&notice.text.render(self.language), room)
                )
            } else {
                ui::truncate(&notice.text.render(self.language), usize::from(area.width))
            };
            let color = if notice.error {
                colors.failure
            } else {
                colors.success
            };
            f.render_widget(
                Paragraph::new(text).style(
                    colors
                        .base()
                        .fg(crate::theme::readable(color, colors.background)),
                ),
                area,
            );
            return;
        }
        if matches!(self.view, View::Terminal)
            && !self.prefix
            && self.sessions.get(self.active).is_some_and(|s| s.exited)
        {
            f.render_widget(
                Paragraph::new(self.t(Text::DisconnectedHint)).style(colors.base()),
                area,
            );
            return;
        }
        if matches!(self.view, View::Terminal)
            && !self.prefix
            && let Some(s) = self.sessions.get(self.active)
            && s.image_error
        {
            let suffix = self.t(Text::PrefixDetails);
            let room = usize::from(area.width).saturating_sub(ui::width(suffix));
            let text = format!(
                "{}{suffix}",
                ui::truncate(&s.status.render(self.language), room)
            );
            f.render_widget(
                Paragraph::new(ui::truncate(&text, usize::from(area.width))).style(
                    colors
                        .base()
                        .fg(crate::theme::readable(colors.failure, colors.background)),
                ),
                area,
            );
            return;
        }
        let state = self.sessions.get(self.active).map(|s| {
            if s.exited {
                self.t(Text::DisconnectedHint).to_owned()
            } else {
                s.status.render(self.language)
            }
        });
        let route = self
            .sessions
            .get(self.active)
            .map(|s| s.route_label.as_str());
        let contexts = [
            route
                .zip(state)
                .map(|(r, s)| format!("{r} · {s} · {}", t.name)),
            route.map(|r| format!("{r} · {}", t.name)),
            Some(t.name.clone()),
        ];
        let required = ui::hint_line(colors, actions, usize::MAX).width();
        let context = if self.prefix {
            None
        } else {
            contexts
                .into_iter()
                .flatten()
                .find(|c| required + ui::width(c) + 3 <= usize::from(area.width))
        };
        let available = usize::from(area.width)
            .saturating_sub(context.as_ref().map_or(0, |c| ui::width(c) + 3));
        f.render_widget(
            Paragraph::new(ui::hint_line(colors, actions, available)).style(colors.base()),
            area,
        );
        if let Some(context) = context {
            let n = ui::width(&context) as u16;
            f.render_widget(
                Paragraph::new(context).style(colors.base().fg(colors.muted)),
                Rect::new(area.right() - n, area.y, n, area.height),
            );
        }
    }
    fn draw_manager(&self, f: &mut Frame, area: Rect) {
        let colors = self.themes[self.theme].ui();
        let matches = self.matches();
        let panel = Panel::new(area, 88, matches.len().clamp(1, 10) + 4);
        panel.render(f, colors);
        ui::heading(
            f,
            panel.heading(),
            colors,
            self.t(Text::Connections),
            &Message::arg(Text::Count, matches.len()).render(self.language),
        );
        let body = panel.body();
        let has_search = body.height >= 2;
        if has_search {
            let search = if self.searching {
                format!(
                    "/ {}",
                    ui::input_window(
                        &self.filter,
                        self.filter.len(),
                        usize::from(body.width).saturating_sub(2)
                    )
                )
            } else if !self.filter.is_empty() {
                format!("/ {}", self.filter)
            } else {
                self.t(Text::Search).into()
            };
            f.render_widget(
                Paragraph::new(ui::truncate(&search, usize::from(body.width))).style(
                    if self.searching {
                        colors.panel().fg(colors.accent)
                    } else {
                        colors.secondary()
                    },
                ),
                ui::row(body, 0),
            );
        }
        let list_area = Rect::new(
            body.x,
            body.y + u16::from(has_search),
            body.width,
            body.height
                .saturating_sub(u16::from(has_search) + u16::from(body.height >= 4)),
        );
        let items: Vec<_> = matches
            .iter()
            .map(|&i| {
                let p = &self.config.connections[i];
                let target = format!(
                    "{}{}{}",
                    if p.user.is_empty() {
                        String::new()
                    } else {
                        format!("{}@", p.user)
                    },
                    p.host,
                    p.port.map_or(String::new(), |port| format!(":{port}"))
                );
                let extra = match &p.route {
                    Route::Direct => self.t(Text::SshConfig).into(),
                    Route::Inherit => self
                        .resolve_proxy(p)
                        .ok()
                        .flatten()
                        .map_or(self.t(Text::SshConfig).into(), |proxy| {
                            format!("HTTP: {}", proxy.name)
                        }),
                    Route::Http(name) => format!("HTTP: {name}"),
                };
                ListRow {
                    name: p.name.clone(),
                    detail: target,
                    extra,
                }
            })
            .collect();
        ui::list(
            f,
            list_area,
            colors,
            &items,
            self.selected,
            self.t(Text::NoConnections),
        );
        ui::hints(
            f,
            panel.footer(),
            colors,
            &[
                ("Enter", self.t(Text::Connect)),
                ("m", self.t(Text::ManualLogin)),
                ("Esc", self.t(Text::Back)),
                ("a", self.t(Text::Add)),
                ("e", self.t(Text::Edit)),
                ("d", self.t(Text::Delete)),
                ("l", self.t(Text::Language)),
                ("?", self.t(Text::More)),
            ],
        );
    }
    fn draw_proxies(&self, f: &mut Frame, area: Rect) {
        let colors = self.themes[self.theme].ui();
        let panel = Panel::new(area, 88, self.config.proxies.len().clamp(1, 10) + 3);
        panel.render(f, colors);
        ui::heading(
            f,
            panel.heading(),
            colors,
            self.t(Text::Proxies),
            self.config
                .default_proxy
                .as_deref()
                .unwrap_or(self.t(Text::DefaultOff)),
        );
        let items: Vec<_> = self
            .config
            .proxies
            .iter()
            .map(|p| ListRow {
                name: format!(
                    "{}{}",
                    if self.config.default_proxy.as_ref() == Some(&p.name) {
                        "★ "
                    } else {
                        ""
                    },
                    p.name
                ),
                detail: p.url.clone(),
                extra: if p.username.is_empty() {
                    self.t(Text::NoAuth).into()
                } else {
                    format!("Basic: {}", p.username)
                },
            })
            .collect();
        ui::list(
            f,
            panel.body(),
            colors,
            &items,
            self.proxy_selected,
            self.t(Text::NoProxies),
        );
        ui::hints(
            f,
            panel.footer(),
            colors,
            &[
                ("Esc", self.t(Text::Back)),
                ("a", self.t(Text::Add)),
                ("e", self.t(Text::Edit)),
                ("d", self.t(Text::Delete)),
                ("Enter", self.t(Text::Default)),
                ("0", self.t(Text::Disable)),
                ("l", self.t(Text::Language)),
            ],
        );
    }
    fn draw_form(&self, f: &mut Frame, area: Rect, form: &Form) {
        let colors = self.themes[self.theme].ui();
        let profile_labels = [
            self.t(Text::Name),
            self.t(Text::Host),
            self.t(Text::User),
            self.t(Text::Port),
            self.t(Text::Identity),
            self.t(Text::Jump),
            self.t(Text::Proxy),
            self.t(Text::SshPassword),
            self.t(Text::RememberPassword),
        ];
        let (title, labels, help): (&str, &[&str], &str) = match form.kind {
            FormKind::Profile { quick, .. } => (
                if quick {
                    self.t(Text::QuickTitle)
                } else {
                    self.t(Text::EditConnection)
                },
                &profile_labels[..if quick { 7 } else { 9 }],
                match form.selected {
                    1 => self.t(Text::HostHelp),
                    2..=5 => self.t(Text::SshHelp),
                    6 => self.t(Text::RouteHelp),
                    7 => self.t(Text::PasswordHelp),
                    8 => self.t(Text::RememberHelp),
                    _ => self.t(Text::NameHelp),
                },
            ),
            FormKind::Proxy { .. } => (
                self.t(Text::EditProxy),
                &[
                    self.t(Text::Name),
                    self.t(Text::HttpUrl),
                    self.t(Text::BasicUser),
                ],
                self.t(Text::ProxyHelp),
            ),
        };
        let wide = area.width >= 64;
        let stride = if wide { 1 } else { 2 };
        let panel = Panel::new(area, 84, labels.len() * stride + 3);
        panel.render(f, colors);
        ui::heading(
            f,
            panel.heading(),
            colors,
            title,
            &format!(
                "{}/{} {}",
                form.selected + 1,
                labels.len(),
                labels[form.selected]
            ),
        );
        let body = panel.body();
        let help_row = body.height > 2;
        let fields_area = Rect::new(
            body.x,
            body.y,
            body.width,
            body.height.saturating_sub(u16::from(help_row)),
        );
        if help_row {
            f.render_widget(
                Paragraph::new(ui::truncate(help, usize::from(body.width)))
                    .style(colors.secondary()),
                ui::row(body, body.height - 1),
            );
        }
        if fields_area.height == 1 {
            let label = format!("{}: ", labels[form.selected]);
            let available = usize::from(fields_area.width).saturating_sub(ui::width(&label));
            let (display, cursor) = form.display_field(form.selected, self.language);
            let value = ui::input_window(&display, cursor, available);
            f.render_widget(
                Paragraph::new(format!(
                    "{}{value}",
                    ui::truncate(&label, usize::from(fields_area.width))
                ))
                .style(colors.selected()),
                fields_area,
            );
        } else {
            let slots = (usize::from(fields_area.height) / stride).max(1);
            let range = ui::visible_range(labels.len(), form.selected, slots);
            for (offset, i) in range.clone().enumerate() {
                let active = i == form.selected;
                let (display, cursor) = form.display_field(i, self.language);
                let style = if active {
                    colors.selected()
                } else {
                    colors.panel()
                };
                let text_style = if active { style } else { colors.secondary() };
                let y = (offset * stride) as u16;
                if wide {
                    let label = format!(
                        "{}{} ",
                        if active { "› " } else { "  " },
                        ui::padded(labels[i], 14)
                    );
                    let available =
                        usize::from(fields_area.width).saturating_sub(ui::width(&label));
                    let value = if active {
                        ui::input_window(&display, cursor, available)
                    } else if display.is_empty() {
                        ui::truncate(self.t(Text::Empty), available)
                    } else {
                        ui::truncate(&display, available)
                    };
                    f.render_widget(
                        Paragraph::new(Line::from(vec![
                            Span::styled(label, style),
                            Span::styled(value, text_style),
                        ]))
                        .style(style),
                        ui::row(fields_area, y),
                    );
                } else {
                    f.render_widget(
                        Paragraph::new(format!(
                            "{}{}",
                            if active { "› " } else { "  " },
                            labels[i]
                        ))
                        .style(if active {
                            style.bold()
                        } else {
                            colors.secondary()
                        }),
                        ui::row(fields_area, y),
                    );
                    let value = if active {
                        ui::input_window(
                            &display,
                            cursor,
                            usize::from(fields_area.width).saturating_sub(2),
                        )
                    } else {
                        ui::truncate(
                            if display.is_empty() {
                                self.t(Text::Empty)
                            } else {
                                &display
                            },
                            usize::from(fields_area.width).saturating_sub(2),
                        )
                    };
                    f.render_widget(
                        Paragraph::new(format!("  {value}")).style(style),
                        ui::row(fields_area, y + 1),
                    );
                }
            }
            // The heading's field counter indicates scrolling; never paint a
            // scroll marker over an input cursor or the end of a long value.
        }
        ui::hints(
            f,
            panel.footer(),
            colors,
            &[
                ("Enter", self.t(Text::Confirm)),
                ("Esc", self.t(Text::Cancel)),
                ("Tab", self.t(Text::Change)),
                ("Ctrl+U", self.t(Text::Clear)),
            ],
        );
    }
    fn draw_languages(&self, f: &mut Frame, area: Rect) {
        let colors = self.themes[self.theme].ui();
        let panel = Panel::new(area, 62, 5);
        panel.render(f, colors);
        ui::heading(
            f,
            panel.heading(),
            colors,
            self.t(Text::Language),
            self.language.code(),
        );
        let items: Vec<_> = Language::ALL
            .iter()
            .map(|language| ListRow {
                name: language.name().into(),
                detail: language.code().into(),
                extra: String::new(),
            })
            .collect();
        ui::list(
            f,
            panel.body(),
            colors,
            &items,
            usize::from(self.language == Language::Zh),
            "",
        );
        ui::hints(
            f,
            panel.footer(),
            colors,
            &[
                ("Enter", self.t(Text::Save)),
                ("Esc", self.t(Text::Restore)),
                ("↑↓", self.t(Text::Preview)),
            ],
        );
    }
    fn theme_layout(&self, area: Rect) -> (Panel, Rect, Rect) {
        let wide = area.width >= 96;
        let rows = self.themes.len().clamp(1, 10);
        let panel = Panel::new(
            area,
            if wide { 108 } else { 70 },
            if wide { rows.max(7) + 2 } else { rows + 8 },
        );
        let body = panel.body();
        let (list_area, preview_area) = if wide {
            let list_width = (body.width / 3).max(22);
            (
                Rect::new(body.x, body.y, list_width, body.height),
                Rect::new(
                    body.x + list_width + 2,
                    body.y,
                    body.width.saturating_sub(list_width + 2),
                    body.height,
                ),
            )
        } else {
            let preview_height = if body.height >= 8 {
                6
            } else if body.height >= 4 {
                2
            } else {
                0
            };
            let list_height = body
                .height
                .saturating_sub(preview_height)
                .max(u16::from(body.height > 0));
            (
                Rect::new(body.x, body.y, body.width, list_height),
                Rect::new(
                    body.x,
                    body.y + list_height,
                    body.width,
                    body.height.saturating_sub(list_height),
                ),
            )
        };
        (panel, list_area, preview_area)
    }
    fn theme_page_rows(&self) -> usize {
        // rows already excludes the tab and status rows.
        let (_, list, _) = self.theme_layout(Rect::new(0, 1, self.cols, self.rows));
        usize::from(list.height).max(1)
    }
    fn draw_themes(&self, f: &mut Frame, area: Rect) {
        let t = &self.themes[self.theme];
        let colors = t.ui();
        let (panel, list_area, preview_area) = self.theme_layout(area);
        panel.render(f, colors);
        let heading = panel.heading();
        let position = format!("{}/{}", self.theme + 1, self.themes.len());
        let width = usize::from(heading.width);
        if width <= ui::width(&position) + 2 {
            f.render_widget(
                Paragraph::new(ui::truncate(&position, width)).style(colors.secondary()),
                heading,
            );
        } else {
            let title = ui::truncate(self.t(Text::ThemeTitle), width - ui::width(&position) - 2);
            let name = ui::truncate(
                &t.name,
                width.saturating_sub(ui::width(&title) + ui::width(&position) + 3),
            );
            let detail = if name.is_empty() {
                position
            } else {
                format!("{position} {name}")
            };
            ui::heading(f, heading, colors, &title, &detail);
        }
        // The picker uses a single name column even on wide preview panels.
        for (offset, i) in
            ui::visible_range(self.themes.len(), self.theme, usize::from(list_area.height))
                .enumerate()
        {
            let style = if i == self.theme {
                colors.selected().bold()
            } else {
                colors.panel()
            };
            f.render_widget(
                Paragraph::new(ui::truncate(
                    &format!(
                        "{}{}",
                        if i == self.theme { "› " } else { "  " },
                        self.themes[i].name
                    ),
                    usize::from(list_area.width),
                ))
                .style(style),
                ui::row(list_area, offset as u16),
            );
        }
        self.draw_preview(f, preview_area);
        ui::hints(
            f,
            panel.footer(),
            colors,
            &[
                ("Enter", self.t(Text::Save)),
                ("Esc", self.t(Text::Restore)),
                ("↑↓", self.t(Text::Preview)),
                ("PgUp/PgDn", self.t(Text::Page)),
                ("Home/End", self.t(Text::FirstLast)),
            ],
        );
    }
    fn draw_preview(&self, f: &mut Frame, area: Rect) {
        let t = &self.themes[self.theme];
        let colors = t.ui();
        let terminal_style = t.style();
        let palette = |i: usize| crate::theme::color(&t.palette[i]).unwrap();
        let lines = [
            Line::from(vec![
                Span::styled("root@server", terminal_style.fg(palette(2)).bold()),
                Span::styled(":~$ ls -l", terminal_style),
            ]),
            Line::from(vec![Span::styled(
                "drwxr-xr-x  中文目录/",
                terminal_style.fg(palette(4)),
            )]),
            Line::from(Span::styled(
                self.t(Text::ProxyConnect),
                colors.panel().fg(colors.failure),
            )),
            Line::from(Span::styled(
                "› production  root@server",
                colors.selected().bold(),
            )),
            Line::from(Span::styled(self.t(Text::PreviewHints), colors.secondary())),
            Line::from(
                t.palette
                    .iter()
                    .map(|c| Span::styled("■ ", terminal_style.fg(crate::theme::color(c).unwrap())))
                    .collect::<Vec<_>>(),
            ),
        ];
        for (i, line) in lines.into_iter().take(usize::from(area.height)).enumerate() {
            // Preview rows have real backgrounds, including the selection surface.
            let style = if i < 2 || i == 5 {
                terminal_style
            } else if i == 3 {
                colors.selected()
            } else {
                colors.panel()
            };
            f.render_widget(Paragraph::new(line).style(style), ui::row(area, i as u16));
        }
    }
    fn text_panel(&self, area: Rect, text: &str, confirm: bool) -> (Panel, Vec<String>) {
        let preferred = if confirm { 66 } else { 78 };
        let initial = Panel::new(area, preferred, 20);
        let lines = ui::wrapped(text, usize::from(initial.inner.width));
        let panel = Panel::new(area, preferred, lines.len().min(18) + 2);
        let lines = ui::wrapped(text, usize::from(panel.inner.width));
        (panel, lines)
    }
    fn text_scroll(&self, old: usize, key: KeyEvent, text: &str) -> usize {
        let (panel, lines) = self.text_panel(Rect::new(0, 0, self.cols, self.rows), text, false);
        let page = usize::from(panel.body().height).max(1);
        let max = lines.len().saturating_sub(page);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => old.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => old.saturating_add(1),
            KeyCode::PageUp => old.saturating_sub(page),
            KeyCode::PageDown => old.saturating_add(page),
            KeyCode::Home => 0,
            KeyCode::End => max,
            _ => old,
        }
        .min(max)
    }
    fn draw_text(
        &self,
        f: &mut Frame,
        area: Rect,
        title: &str,
        text: &str,
        scroll: usize,
        confirm: bool,
    ) {
        let colors = self.themes[self.theme].ui();
        let (panel, lines) = self.text_panel(area, text, confirm);
        panel.render(f, colors);
        let body = panel.body();
        let max = lines.len().saturating_sub(usize::from(body.height));
        let scroll = scroll.min(max);
        ui::heading(
            f,
            panel.heading(),
            colors,
            title,
            if max > 0 {
                self.t(Text::ScrollHint)
            } else {
                ""
            },
        );
        f.render_widget(
            Paragraph::new(
                lines
                    .into_iter()
                    .skip(scroll)
                    .take(usize::from(body.height))
                    .map(Line::from)
                    .collect::<Vec<_>>(),
            )
            .style(colors.panel()),
            body,
        );
        let actions: &[(&str, &str)] = if confirm {
            &[
                ("y", self.t(Text::Confirm)),
                ("n/Esc", self.t(Text::Cancel)),
            ]
        } else {
            &[
                ("Esc", self.t(Text::Back)),
                ("↑↓", self.t(Text::Scroll)),
                ("PgUp/PgDn", self.t(Text::Page)),
            ]
        };
        ui::hints(f, panel.footer(), colors, actions);
    }
}

#[cfg(test)]
#[path = "app_ui_tests.rs"]
mod ui_tests;

#[cfg(test)]
#[path = "credential_tests.rs"]
mod credential_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn theme_cancel_and_unicode_form_editing() {
        let mut app = App::new(
            Config::default(),
            PathBuf::from("unused"),
            crate::theme::builtin(),
            0,
        );
        app.start_themes();
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.theme, 1);
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.theme, 0);
        let mut f = Form::profile(Profile::quick("主机".into()), None, false);
        f.edit(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        f.edit(KeyEvent::new(KeyCode::Char('新'), KeyModifiers::NONE));
        assert_eq!(f.fields[0], "主新机");
    }
}
