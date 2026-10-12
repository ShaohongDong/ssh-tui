use super::*;
use crate::credentials::{CredentialStore, tests::MemoryStore};

fn fixture() -> (tempfile::TempDir, App, Arc<MemoryStore>) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(
        Config::default(),
        dir.path().join("config.toml"),
        crate::theme::builtin(),
        0,
    );
    let store = Arc::new(MemoryStore::default());
    app.credentials = store.clone();
    (dir, app, store)
}

fn create(app: &mut App) -> String {
    let mut form = Form::profile(Profile::quick("example.org".into()), None, false);
    form.fields[2] = "alice".into();
    form.password.push_str(" fixture password 中文 ");
    form.remember_password = true;
    app.submit(&form).unwrap();
    app.config.connections[0].credential_id.clone().unwrap()
}

#[test]
fn password_create_rename_replace_clear_and_username_persist() {
    let (_dir, mut app, store) = fixture();
    let id = create(&mut app);
    let loaded = Config::load(&app.path).unwrap();
    assert_eq!(loaded.connections[0].user, "alice");
    assert_eq!(&*store.get(&id).unwrap(), " fixture password 中文 ");
    let saved = std::fs::read_to_string(&app.path).unwrap();
    assert!(!saved.contains("fixture password"));
    assert!(!saved.contains("中文"));
    let mut form = Form::profile(loaded.connections[0].clone(), Some(0), false);
    assert!(form.password.is_empty());
    form.fields[0] = "renamed".into();
    store.fail.store(true, Ordering::Relaxed);
    app.submit(&form).unwrap(); // Rename neither reads nor rewrites the password.
    assert_eq!(
        app.config.connections[0].credential_id.as_deref(),
        Some(id.as_str())
    );
    store.fail.store(false, Ordering::Relaxed);
    form.password.push_str("replacement");
    app.submit(&form).unwrap();
    let replacement = app.config.connections[0].credential_id.clone().unwrap();
    assert_ne!(replacement, id);
    assert!(store.get(&id).is_err());
    assert_eq!(&*store.get(&replacement).unwrap(), "replacement");
    form = Form::profile(app.config.connections[0].clone(), Some(0), false);
    form.remember_password = false;
    app.submit(&form).unwrap();
    assert!(app.config.connections[0].credential_id.is_none());
    assert!(store.values.lock().unwrap().is_empty());
}

#[test]
fn target_changes_require_new_password_or_explicit_removal() {
    for field in [1, 2, 3] {
        let (_dir, mut app, store) = fixture();
        let id = create(&mut app);
        let mut form = Form::profile(app.config.connections[0].clone(), Some(0), false);
        form.fields[field] = match field {
            1 => "other.example.org",
            2 => "bob",
            _ => "2222",
        }
        .into();
        assert!(app.submit(&form).is_err());
        assert!(store.get(&id).is_ok());
        form.password.push_str("new target password");
        app.submit(&form).unwrap();
        assert!(store.get(&id).is_err());
        assert_eq!(
            &*store
                .get(app.config.connections[0].credential_id.as_ref().unwrap())
                .unwrap(),
            "new target password"
        );
    }
}

#[test]
fn validation_and_vault_failures_leave_config_and_form_unchanged() {
    let (_dir, mut app, store) = fixture();
    let mut form = Form::profile(Profile::quick("example.org".into()), None, false);
    form.remember_password = true;
    assert!(app.submit(&form).is_err());
    form.password.push_str("fixture-password");
    store.fail.store(true, Ordering::Relaxed);
    app.view = View::Form(form);
    assert!(
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .is_err()
    );
    assert!(matches!(app.view, View::Form(_)));
    assert!(!app.path.exists());
    assert!(app.config.connections.is_empty());
    store.fail.store(false, Ordering::Relaxed);
    if let View::Form(form) = &mut app.view {
        form.fields[1] = "bad;host".into();
    }
    assert!(
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .is_err()
    );
    assert!(store.values.lock().unwrap().is_empty());
}

#[test]
fn failed_config_write_rolls_back_new_secret_and_keeps_old_secret() {
    let (dir, mut app, store) = fixture();
    let old = create(&mut app);
    let original = std::fs::read_to_string(&app.path).unwrap();
    let mut form = Form::profile(app.config.connections[0].clone(), Some(0), false);
    form.password.push_str("replacement");
    let path = app.path.clone();
    app.path = dir.path().to_owned(); // rename cannot replace a directory.
    assert!(app.submit(&form).is_err());
    assert_eq!(store.values.lock().unwrap().len(), 1);
    assert!(store.get(&old).is_ok());
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
}

#[test]
fn cleanup_failure_is_persistent_and_next_save_retries() {
    let (_dir, mut app, store) = fixture();
    let old = create(&mut app);
    store.fail.store(true, Ordering::Relaxed);
    app.confirm(Confirm::DeleteProfile(0)).unwrap();
    assert!(app.config.connections.is_empty());
    assert_eq!(
        Config::load(&app.path).unwrap().pending_credential_cleanup,
        vec![old.clone()]
    );
    assert!(app.notice.as_ref().unwrap().error);
    store.fail.store(false, Ordering::Relaxed);
    app.persist(app.config.clone()).unwrap();
    assert!(store.get(&old).is_err());
    assert!(
        Config::load(&app.path)
            .unwrap()
            .pending_credential_cleanup
            .is_empty()
    );
}

#[test]
fn cancel_discards_password_without_touching_vault_and_quick_has_no_password_fields() {
    let (_dir, mut app, store) = fixture();
    let old = create(&mut app);
    let original = std::fs::read_to_string(&app.path).unwrap();
    let mut form = Form::profile(app.config.connections[0].clone(), Some(0), false);
    form.password.push_str("unsaved replacement");
    app.view = View::Form(form);
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(std::fs::read_to_string(&app.path).unwrap(), original);
    assert_eq!(&*store.get(&old).unwrap(), " fixture password 中文 ");
    assert_eq!(
        Form::profile(Profile::quick("host".into()), None, true)
            .fields
            .len(),
        7
    );
}

#[test]
fn password_paste_masks_unicode_cursor_and_space_toggles_save() {
    let mut form = Form::profile(Profile::quick("host".into()), None, false);
    form.selected = 7;
    form.paste("中 文 pw").unwrap();
    assert_eq!(form.display_field(7, Language::Zh), ("******".into(), 6));
    form.edit(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(form.display_field(7, Language::En).1, 5);
    assert!(form.paste("bad\npassword").is_err());
    form.selected = 8;
    form.edit(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert!(form.remember_password);
}
