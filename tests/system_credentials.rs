use ssh_tui::credentials::{CredentialStore, SystemStore, new_id};

#[test]
#[ignore = "requires an isolated, unlocked native vault; run via e2e_credentials.py"]
fn native_vault_round_trip_and_missing_entry() {
    let id = new_id();
    struct Cleanup(String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = SystemStore.delete(&self.0);
        }
    }
    let _cleanup = Cleanup(id.clone());
    assert!(SystemStore.get(&id).is_err());
    SystemStore
        .set(&id, " fixture native password 中文 ")
        .unwrap();
    assert_eq!(
        &*SystemStore.get(&id).unwrap(),
        " fixture native password 中文 "
    );
    SystemStore.set(&id, "replacement").unwrap();
    assert_eq!(&*SystemStore.get(&id).unwrap(), "replacement");
    SystemStore.delete(&id).unwrap();
    SystemStore.delete(&id).unwrap();
    assert!(SystemStore.get(&id).is_err());
}
