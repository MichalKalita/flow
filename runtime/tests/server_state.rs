use flow_runtime::server_state::ServerState;
use serde_json::json;
use std::{fs, sync::Arc};

struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("flow-server-state-{}", uuid::Uuid::new_v4())))
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn identity_keys_settings_and_encrypted_secrets_survive_restart() {
    let directory = Directory::new();
    let secret = "unique-project-secret-not-in-plaintext-0123456789";
    let first = ServerState::open(&directory.0).unwrap();
    let identity = first.status().unwrap();
    first.set_secret("contacts", "signing", secret).unwrap();
    first.set_secret("contacts", "short", "abc").unwrap();
    first
        .set_secret("other", "signing", "different-project-key")
        .unwrap();
    let current = first.status().unwrap();
    first
        .update_settings(
            &json!({"revision":current["revision"],"name":"Office server","backup_keep":3}),
        )
        .unwrap();
    let token = first
        .enroll("Office server", "a strong owner password")
        .unwrap();
    assert_eq!(first.login("a strong owner password").unwrap(), token);
    assert!(first.login("wrong password").is_err());
    assert!(first.enroll("Other", "different strong password").is_err());
    drop(first);
    let second = ServerState::open(&directory.0).unwrap();
    let restored = second.status().unwrap();
    assert_eq!(restored["instance"], identity["instance"]);
    assert_eq!(restored["public_key"], identity["public_key"]);
    assert_eq!(restored["name"], "Office server");
    assert_eq!(restored["backup_keep"], 3);
    assert_eq!(restored["setup_complete"], true);
    assert_eq!(
        second.secret("contacts", "signing").unwrap().as_deref(),
        Some(secret)
    );
    assert_ne!(
        second.secret("other", "signing").unwrap().as_deref(),
        Some(secret)
    );
    assert_eq!(second.login("a strong owner password").unwrap(), token);
    let previews = restored["secrets"].as_array().unwrap();
    assert_eq!(
        previews
            .iter()
            .find(|p| p["name"] == "contacts/short")
            .unwrap()["preview"],
        "••••••••"
    );
    assert!(!restored.to_string().contains(secret));
    for file in ["server.sqlite", "server.sqlite-wal"] {
        if let Ok(bytes) = fs::read(directory.0.join(file)) {
            assert!(
                !bytes
                    .windows(secret.len())
                    .any(|window| window == secret.as_bytes())
            );
            assert!(
                !bytes
                    .windows(token.len())
                    .any(|window| window == token.as_bytes())
            );
        }
    }
}

#[test]
fn stale_or_invalid_changes_leave_working_settings_intact() {
    let directory = Directory::new();
    let state = ServerState::open(&directory.0).unwrap();
    let initial = state.status().unwrap();
    let revision = initial["revision"].as_i64().unwrap();
    state
        .update_settings(&json!({"revision":revision,"name":"New name"}))
        .unwrap();
    assert_eq!(
        state
            .update_settings(&json!({"revision":revision,"name":"Stale"}))
            .unwrap_err()
            .code,
        "conflict"
    );
    let current = state.status().unwrap();
    assert!(
        state
            .update_settings(
                &json!({"revision":current["revision"],"name":"Must roll back","backup_keep":0})
            )
            .is_err()
    );
    assert_eq!(state.status().unwrap(), current);
    assert!(
        state
            .update_settings(&json!({"revision":current["revision"],"unknown":"x"}))
            .is_err()
    );
    assert_eq!(state.status().unwrap(), current);
}

#[test]
fn missing_or_wrong_keys_never_replace_existing_encrypted_state() {
    let directory = Directory::new();
    let state = ServerState::open(&directory.0).unwrap();
    state.set_secret("one", "key", "persistent secret").unwrap();
    drop(state);
    let keys = fs::read(directory.0.join("server.keys")).unwrap();
    fs::remove_file(directory.0.join("server.keys")).unwrap();
    assert!(ServerState::open(&directory.0).is_err());
    assert!(!directory.0.join("server.keys").exists());
    fs::write(directory.0.join("server.keys"), b"invalid").unwrap();
    assert!(ServerState::open(&directory.0).is_err());
    fs::write(directory.0.join("server.keys"), keys).unwrap();
    assert_eq!(
        ServerState::open(&directory.0)
            .unwrap()
            .secret("one", "key")
            .unwrap()
            .as_deref(),
        Some("persistent secret")
    );
}

#[test]
fn concurrent_first_start_creates_one_identity_and_one_admin_credential() {
    let directory = Arc::new(Directory::new());
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let directory = directory.clone();
            std::thread::spawn(move || {
                let state = ServerState::open(&directory.0).unwrap();
                (
                    state.status().unwrap()["instance"].clone(),
                    state.admin_token().unwrap(),
                )
            })
        })
        .collect();
    let identities: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert!(identities.iter().all(|identity| identity == &identities[0]));
}

#[test]
fn orphaned_key_initialization_can_complete_without_rotating_keys() {
    let directory = Directory::new();
    let state = ServerState::open(&directory.0).unwrap();
    let identity = state.status().unwrap();
    drop(state);
    for file in ["server.sqlite", "server.sqlite-wal", "server.sqlite-shm"] {
        let _ = fs::remove_file(directory.0.join(file));
    }
    let recovered = ServerState::open(&directory.0).unwrap().status().unwrap();
    assert_eq!(identity["instance"], recovered["instance"]);
    assert_eq!(identity["public_key"], recovered["public_key"]);
}

#[test]
fn password_protected_recovery_unlocks_state_on_a_fresh_server_directory() {
    let directory = Directory::new();
    let restored = Directory::new();
    let state = ServerState::open(&directory.0).unwrap();
    state
        .enroll("Recovery test", "fictional recovery password")
        .unwrap();
    state
        .set_secret("contacts", "signing", "recoverable project credential")
        .unwrap();
    std::fs::create_dir_all(&restored.0).unwrap();
    state.snapshot(&restored.0.join("server.sqlite")).unwrap();
    let package = std::fs::read(directory.0.join("recovery.keys")).unwrap();
    assert!(flow_runtime::server_state::unlock_recovery(&package, "incorrect").is_err());
    let keys = flow_runtime::server_state::unlock_recovery(&package, "fictional recovery password")
        .unwrap();
    std::fs::write(restored.0.join("server.keys"), keys).unwrap();
    let recovered = ServerState::open(&restored.0).unwrap();
    assert_eq!(
        recovered.status().unwrap()["instance"],
        state.status().unwrap()["instance"]
    );
    assert_eq!(
        recovered.secret("contacts", "signing").unwrap().as_deref(),
        Some("recoverable project credential")
    );
}
