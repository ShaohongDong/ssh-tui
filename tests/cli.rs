use std::process::Command;

#[test]
fn cli_help_and_argument_errors_are_available_without_a_tty() {
    let binary = env!("CARGO_BIN_EXE_ssh-tui");
    let help = Command::new(binary).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--proxy-user"));
    for args in [
        vec!["--port", "22"],
        vec!["host", "--proxy-user", "user"],
        vec!["host", "--proxy", "http://localhost:7890", "--no-proxy"],
    ] {
        assert!(
            !Command::new(binary)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

#[test]
fn delivered_config_and_custom_theme_are_loadable() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    ssh_tui::config::Config::load(&root.join("examples/config.toml"))
        .unwrap()
        .validate()
        .unwrap();
    let themes = ssh_tui::theme::load(&root.join("examples/themes")).unwrap();
    assert!(themes.iter().any(|t| t.name == "Amber"));
}

#[test]
fn language_precedence_and_startup_errors_work_without_a_tty() {
    let binary = env!("CARGO_BIN_EXE_ssh-tui");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    for (config, cli, expected) in [
        ("", None, "Run in an interactive terminal"),
        ("language = 'zh'", None, "请在交互式终端中运行"),
        (
            "language = 'zh'",
            Some("en"),
            "Run in an interactive terminal",
        ),
        ("language = 'en'", Some("zh"), "请在交互式终端中运行"),
    ] {
        std::fs::write(&path, config).unwrap();
        let mut command = Command::new(binary);
        command.arg("--config").arg(&path);
        if let Some(language) = cli {
            command.args(["--language", language]);
        }
        let result = command.output().unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains(expected));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), config);
    }
    std::fs::write(&path, "invalid toml").unwrap();
    let result = Command::new(binary)
        .arg("--config")
        .arg(&path)
        .args(["--language", "zh"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&result.stderr).contains("配置 TOML 格式错误"));
    assert!(
        !Command::new(binary)
            .args(["--language", "fr"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let help = Command::new(binary)
        .args(["--language", "zh", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("--language") && help.contains("[possible values: en, zh]"));
    assert!(!help.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)));
}
