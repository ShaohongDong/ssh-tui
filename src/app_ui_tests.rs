use super::*;
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

fn app(count: usize) -> App {
    let mut config = Config {
        connections: (0..count)
            .map(|i| Profile {
                name: if count > 2 {
                    format!("生产服务器-{i:02}-很长的连接名称")
                } else {
                    ["production", "staging"][i].into()
                },
                host: format!("10.0.0.{}", i + 8),
                user: "root".into(),
                port: None,
                identity: String::new(),
                jump: String::new(),
                route: Route::Inherit,
            })
            .collect(),
        ..Config::default()
    };
    config.proxies.push(Proxy {
        name: "office".into(),
        url: "http://127.0.0.1:7890".into(),
        username: "alice".into(),
    });
    config.default_proxy = Some("office".into());
    App::new(config, PathBuf::from("unused"), crate::theme::builtin(), 0)
}

fn render(app: &mut App, w: u16, h: u16) -> Buffer {
    app.dimensions(w, h).unwrap();
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    terminal.backend().buffer().clone()
}

fn text(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|y| {
            let mut line = String::new();
            let mut x = 0;
            while x < buffer.area.width {
                let c = &buffer[(x, y)];
                line.push_str(c.symbol());
                x += ui::width(c.symbol()).max(1) as u16;
            }
            line.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_ui_contrast(buffer: &Buffer) {
    for c in &buffer.content {
        if c.symbol().trim().is_empty() || "╭╮╰╯─│".contains(c.symbol()) {
            continue;
        }
        assert!(
            crate::theme::contrast(c.fg, c.bg) >= 4.5,
            "{:?} {:?} on {:?}",
            c.symbol(),
            c.fg,
            c.bg
        );
    }
}

#[test]
fn ui_snapshots_cover_responsive_views() {
    for language in Language::ALL {
        for (w, h) in [(80, 24), (120, 36), (48, 12), (24, 6)] {
            let mut app = app(2);
            app.language = language;
            let mut result = String::new();
            let mut capture = |label: &str, app: &mut App, check_contrast: bool| {
                let buffer = render(app, w, h);
                if check_contrast {
                    assert_ui_contrast(&buffer);
                }
                result.push_str(&format!("[{label} {w}x{h}]\n{}\n\n", text(&buffer)));
            };
            capture("two-connections", &mut app, true);
            app.config.connections.clear();
            capture("empty-connections", &mut app, true);
            app = self::app(24);
            app.language = language;
            app.selected = 22;
            capture("many-long-connections", &mut app, true);
            app.view = View::Proxies;
            capture("proxy", &mut app, true);
            let mut form = Form::profile(app.config.connections[0].clone(), Some(0), false);
            form.selected = 4;
            form.fields[4] =
                "/home/开发者/very/long/directory/path/with/multiple/components/id_ed25519".into();
            form.cursors[4] = form.fields[4].len();
            app.view = View::Form(form);
            capture("long-input-form", &mut app, true);
            app.view = View::Confirm(Confirm::DeleteProfile(0));
            capture("confirmation", &mut app, true);
            app.start_help();
            capture("help-start", &mut app, true);
            app.on_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE))
                .unwrap();
            capture("help-end", &mut app, true);
            app.start_themes();
            capture("theme-preview", &mut app, false);
            app.view = View::Manager;
            app.start_languages();
            capture("language-picker", &mut app, true);
            let result = result.trim_end_matches('\n').to_owned() + "\n";
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/snapshots/ui-{}-{w}x{h}.txt",
                language.code()
            ));
            if std::env::var_os("UPDATE_UI_SNAPSHOTS").is_some() {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, &result).unwrap();
            }
            assert_eq!(
                result,
                std::fs::read_to_string(path).expect("UI snapshot missing; review before updating")
            );
        }
    }
    if std::env::var_os("UPDATE_UI_SNAPSHOTS").is_some() {
        export_preview_artifacts();
    }
}

#[test]
fn compact_manager_and_form_keep_focus_and_controls() {
    let mut app = app(2);
    for language in Language::ALL {
        app.language = language;
        app.view = View::Manager;
        let panel = Panel::new(Rect::new(0, 1, 80, 22), 88, 6);
        assert_eq!(panel.outer.height, 8);
        let snapshot = text(&render(&mut app, 80, 24));
        assert!(snapshot.contains("production") && snapshot.contains("staging"));
        assert!(
            snapshot.contains(&format!("Enter {}", app.t(Text::Connect)))
                && snapshot.contains(&format!("Esc {}", app.t(Text::Back)))
        );
        assert!(snapshot.contains("HTTP: office"));
        for (w, h) in [(48, 12), (24, 6), (24, 5)] {
            for selected in 0..7 {
                let mut form = Form::profile(Profile::quick("target".into()), None, false);
                form.selected = selected;
                form.fields[selected] = "中文很长的输入值-and-a-long-path".into();
                form.cursors[selected] = form.fields[selected].len();
                app.view = View::Form(form);
                let snapshot = text(&render(&mut app, w, h));
                assert!(
                    snapshot.contains('▏'),
                    "{w}x{h} field {selected}\n{snapshot}"
                );
                assert!(
                    snapshot.contains(&format!("Enter {}", app.t(Text::Confirm)))
                        && snapshot.contains(&format!("Esc {}", app.t(Text::Cancel))),
                    "{snapshot}"
                );
            }
        }
    }
}

#[test]
fn all_themes_render_readable_ui_and_tiny_windows_do_not_panic() {
    let mut app = app(2);
    for language in Language::ALL {
        app.language = language;
        for theme in 0..app.themes.len() {
            app.theme = theme;
            app.view = View::Manager;
            assert_ui_contrast(&render(&mut app, 80, 24));
            let mut form = Form::profile(app.config.connections[0].clone(), Some(0), false);
            form.selected = 6;
            app.view = View::Form(form);
            assert_ui_contrast(&render(&mut app, 80, 24));
        }
    }
    let views = [
        View::Manager,
        View::Proxies,
        View::Form(Form::profile(Profile::quick("target".into()), None, false)),
        View::Confirm(Confirm::Quit),
        View::Themes { previous: 0 },
        View::Help {
            scroll: 1000,
            previous: Box::new(View::Manager),
        },
    ];
    for (w, h) in [(1, 1), (2, 2), (3, 3), (10, 4), (24, 6)] {
        for view in &views {
            app.view = view.clone();
            render(&mut app, w, h);
        }
    }
}

#[test]
fn error_details_and_help_scroll_then_restore_the_previous_view() {
    let mut app = app(2);
    app.dimensions(48, 12).unwrap();
    app.view = View::Proxies;
    app.start_help();
    app.on_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(app.view, View::Help { scroll, .. } if scroll > 0));
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(app.view, View::Proxies));
    app.error(
        (0..80)
            .map(|i| format!("错误详情 {i}\n"))
            .collect::<String>(),
    );
    app.on_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE))
        .unwrap();
    app.on_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE))
        .unwrap();
    let snapshot = text(&render(&mut app, 48, 12));
    assert!(snapshot.contains("错误详情 79"));
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(app.view, View::Proxies) && app.notice.is_some());
    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    assert!(app.notice.is_none());
}

fn export_preview_artifacts() {
    let mut app = app(2);
    for (name, theme) in [("dark", 0), ("light", 5)] {
        app.theme = theme;
        let buffer = render(&mut app, 80, 24);
        let mut output = "80\t24\n".to_string();
        let hex = |c: ratatui::style::Color, fallback: ratatui::style::Color| match c {
            ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            _ => match fallback {
                ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
                _ => unreachable!(),
            },
        };
        for y in 0..24 {
            for x in 0..80 {
                let c = &buffer[(x, y)];
                output.push_str(&format!(
                    "{x}\t{y}\t{}\t{}\t{}\t{}\n",
                    hex(c.fg, app.themes[theme].fg()),
                    hex(c.bg, app.themes[theme].bg()),
                    c.modifier.bits(),
                    c.symbol()
                ));
            }
        }
        std::fs::create_dir_all("target").unwrap();
        std::fs::write(format!("target/ui-{name}.tsv"), output).unwrap();
    }
}

#[test]
fn language_picker_restores_view_and_only_explicit_save_persists_override() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut app = app(2).with_language(Language::Zh);
    app.path = path.clone();
    app.persist(app.config.clone()).unwrap();
    assert_eq!(Config::load(&path).unwrap().language, Language::En);
    assert_eq!(app.language, Language::Zh);
    for previous in [View::Manager, View::Proxies, View::Terminal] {
        app.view = previous.clone();
        app.start_languages();
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.language, Language::En);
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.language, Language::Zh);
        assert_eq!(
            std::mem::discriminant(&app.view),
            std::mem::discriminant(&previous)
        );
    }
    app.start_languages();
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(app.view, View::Terminal));
    assert_eq!(Config::load(&path).unwrap().language, Language::Zh);
    let saved = app.notice.as_ref().unwrap().text.clone();
    assert_eq!(saved.render(Language::En), "Settings saved");
    assert_eq!(saved.render(Language::Zh), "配置已保存");
}

#[test]
fn failed_language_save_keeps_picker_and_cancel_restores_language() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file");
    std::fs::write(&file, "keep").unwrap();
    let mut app = app(2);
    app.path = file.join("config.toml");
    app.view = View::Proxies;
    app.start_languages();
    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    let error = app
        .on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap_err();
    app.error(Diagnostic::capture(&error));
    assert!(matches!(app.view, View::Languages { .. }));
    assert_eq!(app.config.language, Language::En);
    assert_eq!(app.language, Language::Zh);
    assert!(text(&render(&mut app, 80, 24)).contains("无法"));
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.language, Language::En);
    assert!(matches!(app.view, View::Proxies));
    assert_eq!(std::fs::read_to_string(file).unwrap(), "keep");
}
