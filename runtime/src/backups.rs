use crate::{
    Error, Result,
    engine::{Config, Runtime},
    projects::Projects,
    server_state::{ServerState, unlock_recovery},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

fn storage(_: impl std::fmt::Display) -> Error {
    Error::new("storage", "Backup storage is unavailable")
}
fn invalid() -> Error {
    Error::new(
        "invalid_input",
        "Backup is incomplete, incompatible, or damaged",
    )
}
fn private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(storage)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(storage)?;
    }
    Ok(())
}
fn sync_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(storage)?;
    }
    fs::File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(storage)
}
fn digest(path: &Path) -> Result<(u64, String)> {
    if fs::symlink_metadata(path)
        .map_err(storage)?
        .file_type()
        .is_symlink()
    {
        return Err(invalid());
    }
    let mut file = fs::File::open(path).map_err(storage)?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut bytes = [0u8; 65536];
    loop {
        let count = file.read(&mut bytes).map_err(storage)?;
        if count == 0 {
            break;
        }
        total = total.checked_add(count as u64).ok_or_else(invalid)?;
        hash.update(&bytes[..count]);
    }
    Ok((total, format!("{:x}", hash.finalize())))
}
fn record(root: &Path, relative: &str) -> Result<Value> {
    let path = root.join(relative);
    sync_file(&path)?;
    let (bytes, sha256) = digest(&path)?;
    Ok(json!({"path":relative,"bytes":bytes,"sha256":sha256}))
}
fn root(projects: &Projects) -> Result<PathBuf> {
    let server = projects
        .server
        .as_ref()
        .ok_or_else(|| Error::new("configuration", "Persistent server settings are required"))?;
    Ok(server
        .directory()
        .parent()
        .unwrap_or(Path::new("."))
        .join("backups"))
}
fn directory(projects: &Projects, id: &str) -> Result<PathBuf> {
    uuid::Uuid::parse_str(id).map_err(|_| invalid())?;
    Ok(root(projects)?.join(id))
}

pub fn list(projects: &Projects) -> Result<Value> {
    let root = root(projects)?;
    if !root.exists() {
        return Ok(json!({"backups":[]}));
    }
    let mut backups = Vec::new();
    for entry in fs::read_dir(root).map_err(storage)? {
        let entry = entry.map_err(storage)?;
        if !entry.file_type().map_err(storage)?.is_dir()
            || uuid::Uuid::parse_str(&entry.file_name().to_string_lossy()).is_err()
        {
            continue;
        }
        if let Ok(manifest) = manifest(&entry.path()) {
            backups.push(json!({"id":entry.file_name().to_string_lossy(),"created_at":manifest["created_at"],"server":manifest["server"],"projects":manifest["projects"],"bytes":manifest["files"].as_array().unwrap().iter().filter_map(|file|file["bytes"].as_u64()).fold(0u64,u64::saturating_add)}));
        }
    }
    backups.sort_by(|a, b| b["created_at"].as_str().cmp(&a["created_at"].as_str()));
    Ok(json!({"backups":backups}))
}

pub fn create(projects: &Projects) -> Result<Value> {
    let _maintenance = projects
        .maintenance
        .try_lock()
        .map_err(|_| Error::new("conflict", "Project maintenance is already running"))?;
    let (_, data) = projects.roots()?;
    let server = projects
        .server
        .as_ref()
        .ok_or_else(|| Error::new("configuration", "Persistent server settings are required"))?;
    if server.status()?["setup_complete"] != true
        || !server.directory().join("recovery.keys").is_file()
    {
        return Err(Error::new(
            "configuration",
            "Set up an owner password before creating recoverable backups",
        ));
    }
    if projects
        .list()
        .as_array()
        .is_some_and(|entries| entries.iter().any(|entry| entry["active"] != true))
    {
        return Err(Error::new(
            "conflict",
            "Resolve unavailable projects before backing up the complete server",
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let root = root(projects)?;
    private_directory(&root)?;
    let staging = root.join(format!(".{id}.pending"));
    private_directory(&staging)?;
    let result: Result<Value> = (|| {
        private_directory(&staging.join("projects"))?;
        private_directory(&staging.join("databases"))?;
        private_directory(&staging.join("server"))?;
        let active = projects.active();
        // Holding every project lock defines one capture boundary and prevents
        // a completed write from being split from its audit or allocation state.
        let guards = active
            .iter()
            .map(|(_, runtime)| runtime.lock().map_err(storage))
            .collect::<Result<Vec<_>>>()?;
        let mut files = Vec::new();
        let mut names = Vec::new();
        let mut secrets = Vec::new();
        for ((name, _), runtime) in active.iter().zip(&guards) {
            let cache: Value = serde_json::from_slice(
                &fs::read(data.join(format!("{name}-active.json"))).map_err(storage)?,
            )?;
            let source = cache["source"].as_str().ok_or_else(invalid)?;
            let mut settings = cache["manifest"].clone();
            let config = runtime.configuration();
            for (alias, key) in config.jwt_keys {
                let secret_name = format!("backup-jwt-{alias}");
                let value = String::from_utf8(key).map_err(|_| invalid())?;
                secrets.push((name.clone(), secret_name.clone(), value));
                settings["jwt_secrets"][alias] = json!({"secret":secret_name});
            }
            for (actor, credential) in config.event_credentials {
                let secret_name = format!("backup-event-{}", actor.replace(':', "-"));
                let value = credential
                    .strip_prefix("ApiKey ")
                    .ok_or_else(invalid)?
                    .to_owned();
                secrets.push((name.clone(), secret_name.clone(), value));
                settings["event_credentials"][actor] = json!({"secret":secret_name});
            }
            private_directory(&staging.join("projects").join(name))?;
            let program_path = format!("projects/{name}/application.flow");
            fs::write(staging.join(&program_path), source).map_err(storage)?;
            files.push(record(&staging, &program_path)?);
            let settings_path = format!("projects/{name}/project.json");
            fs::write(staging.join(&settings_path), serde_json::to_vec(&settings)?)
                .map_err(storage)?;
            files.push(record(&staging, &settings_path)?);
            let database_path = format!("databases/{name}.sqlite");
            runtime.snapshot_to(&staging.join(&database_path))?;
            files.push(record(&staging, &database_path)?);
            names.push(name.clone());
        }
        server.snapshot_with_secrets(&staging.join("server/server.sqlite"), &secrets)?;
        files.push(record(&staging, "server/server.sqlite")?);
        fs::copy(
            server.directory().join("recovery.keys"),
            staging.join("server/recovery.keys"),
        )
        .map_err(storage)?;
        files.push(record(&staging, "server/recovery.keys")?);
        let mut manifest = json!({"format":1,"runtime_version":env!("CARGO_PKG_VERSION"),"id":id,"created_at":chrono::Utc::now().to_rfc3339(),"server":server.status()?["instance"],"projects":names,"files":files});
        manifest["signature"] =
            json!(server.sign_backup_manifest(&serde_json::to_vec(&manifest)?)?);
        fs::write(
            staging.join("manifest.json"),
            serde_json::to_vec(&manifest)?,
        )
        .map_err(storage)?;
        sync_file(&staging.join("manifest.json"))?;
        fs::File::open(&staging)
            .and_then(|file| file.sync_all())
            .map_err(storage)?;
        let destination = root.join(&id);
        fs::rename(&staging, &destination).map_err(storage)?;
        fs::File::open(&root)
            .and_then(|file| file.sync_all())
            .map_err(storage)?;
        Ok(manifest)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    let manifest = result?;
    let keep = server.status()?["backup_keep"].as_u64().unwrap_or(7) as usize;
    let retained = list(projects)?;
    for old in retained["backups"].as_array().unwrap().iter().skip(keep) {
        if let Some(id) = old["id"].as_str() {
            fs::remove_dir_all(directory(projects, id)?).map_err(storage)?;
        }
    }
    Ok(json!({"id":id,"created_at":manifest["created_at"],"projects":manifest["projects"]}))
}

fn manifest(path: &Path) -> Result<Value> {
    let manifest_path = path.join("manifest.json");
    if fs::metadata(&manifest_path).map_err(storage)?.len() > 128 * 1024 {
        return Err(invalid());
    }
    let manifest: Value = serde_json::from_slice(&fs::read(manifest_path).map_err(storage)?)
        .map_err(|_| invalid())?;
    if manifest["format"] != 1
        || manifest["runtime_version"] != env!("CARGO_PKG_VERSION")
        || !manifest["files"].is_array()
        || !manifest["projects"].is_array()
    {
        return Err(invalid());
    }
    if manifest["projects"].as_array().unwrap().len() > 32
        || manifest["files"].as_array().unwrap().len() > 100
    {
        return Err(invalid());
    }
    Ok(manifest)
}
fn safe_path(value: &str) -> bool {
    !value.is_empty()
        && !Path::new(value).is_absolute()
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        && !value.contains('\\')
}

pub struct RestoredBackup {
    pub directory: PathBuf,
    pub manifest: Value,
    cleanup: bool,
}
impl Drop for RestoredBackup {
    fn drop(&mut self) {
        if self.cleanup {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}

pub fn prepare(path: &Path, password: &str, parent: &Path) -> Result<RestoredBackup> {
    let manifest = manifest(path)?;
    fs::create_dir_all(parent).map_err(storage)?;
    let directory = parent.join(format!(".restore-{}", uuid::Uuid::new_v4()));
    private_directory(&directory)?;
    let restored = RestoredBackup {
        directory,
        manifest,
        cleanup: true,
    };
    let project_names = restored.manifest["projects"]
        .as_array()
        .ok_or_else(invalid)?;
    if project_names
        .iter()
        .filter_map(Value::as_str)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != project_names.len()
    {
        return Err(invalid());
    }
    let mut required = std::collections::BTreeSet::from([
        "server/server.sqlite".to_owned(),
        "server/recovery.keys".to_owned(),
    ]);
    for project in restored.manifest["projects"]
        .as_array()
        .ok_or_else(invalid)?
    {
        let name = project
            .as_str()
            .filter(|name| crate::projects::name_valid(name))
            .ok_or_else(invalid)?;
        for filename in ["application.flow", "project.json"] {
            required.insert(format!("projects/{name}/{filename}"));
        }
        required.insert(format!("databases/{name}.sqlite"));
    }
    if required.len()
        != restored.manifest["files"]
            .as_array()
            .ok_or_else(invalid)?
            .len()
    {
        return Err(invalid());
    }

    for file in restored.manifest["files"].as_array().unwrap() {
        let relative = file["path"].as_str().ok_or_else(invalid)?;
        if !safe_path(relative) || !required.remove(relative) {
            return Err(invalid());
        }
        let original = path.join(relative);
        let (bytes, sha256) = digest(&original)?;
        if file["bytes"] != bytes || file["sha256"] != sha256 {
            return Err(invalid());
        }
        let destination = restored.directory.join(relative);
        private_directory(destination.parent().ok_or_else(invalid)?)?;
        let mut input = fs::File::open(original).map_err(storage)?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options.open(&destination).map_err(storage)?;
        std::io::copy(&mut input, &mut output).map_err(storage)?;
        sync_file(&destination)?;
        let copied = digest(&destination)?;
        if copied != (bytes, sha256) {
            return Err(invalid());
        }
    }
    let keys = unlock_recovery(
        &fs::read(restored.directory.join("server/recovery.keys")).map_err(storage)?,
        password,
    )?;
    let signature = restored.manifest["signature"]
        .as_str()
        .ok_or_else(invalid)?;
    let mut unsigned = restored.manifest.clone();
    unsigned
        .as_object_mut()
        .ok_or_else(invalid)?
        .remove("signature");
    crate::server_state::verify_backup_manifest(&keys, &serde_json::to_vec(&unsigned)?, signature)?;
    crate::server_state::write_private(&restored.directory.join("server/server.keys"), &keys)?;
    let server = ServerState::open(&restored.directory.join("server"))?;
    if server.status()?["instance"] != restored.manifest["server"] {
        return Err(invalid());
    }
    for project in restored.manifest["projects"].as_array().unwrap() {
        let name = project.as_str().ok_or_else(invalid)?;
        if !crate::projects::name_valid(name) {
            return Err(invalid());
        }
        let source_path = restored
            .directory
            .join("projects")
            .join(name)
            .join("application.flow");
        if fs::metadata(&source_path).map_err(storage)?.len() > 4 * 1024 * 1024 {
            return Err(invalid());
        }
        let source = fs::read_to_string(&source_path).map_err(storage)?;
        let config_value: Value = serde_json::from_slice(
            &fs::read(
                restored
                    .directory
                    .join("projects")
                    .join(name)
                    .join("project.json"),
            )
            .map_err(storage)?,
        )?;
        let config = restored_config(name, &config_value, &server)?;
        let runtime = Runtime::open(
            &source,
            restored
                .directory
                .join("databases")
                .join(format!("{name}.sqlite"))
                .to_str()
                .ok_or_else(invalid)?,
            config,
        )?;
        runtime.validate_storage()?;
    }
    Ok(restored)
}

fn restored_config(name: &str, settings: &Value, server: &ServerState) -> Result<Config> {
    let mut config = Config {
        base_path: format!("/{name}"),
        ..Config::default()
    };
    config.test_seeds = match settings.get("test_seeds") {
        None => false,
        Some(value) => value.as_bool().ok_or_else(invalid)?,
    };
    for (section, jwt) in [("jwt_secrets", true), ("event_credentials", false)] {
        if let Some(values) = settings.get(section) {
            for (alias, reference) in values.as_object().ok_or_else(invalid)? {
                let secret = reference["secret"].as_str().ok_or_else(invalid)?;
                let value = server.secret(name, secret)?.ok_or_else(invalid)?;
                if jwt {
                    config.jwt_keys.insert(alias.clone(), value.into_bytes());
                } else {
                    config
                        .event_credentials
                        .insert(alias.clone(), format!("ApiKey {value}"));
                }
            }
        }
    }
    Ok(config)
}

pub fn verify(projects: &Projects, id: &str, password: &str) -> Result<Value> {
    let restored = prepare(&directory(projects, id)?, password, &root(projects)?)?;
    Ok(
        json!({"id":id,"verified":true,"projects":restored.manifest["projects"],"created_at":restored.manifest["created_at"]}),
    )
}

pub fn restore_fresh(path: &Path, password: &str, target: &Path) -> Result<Value> {
    if target.exists() && fs::read_dir(target).map_err(storage)?.next().is_some() {
        return Err(Error::new("conflict", "Restore destination must be empty"));
    }
    let parent = target.parent().unwrap_or(Path::new("."));
    let restored = prepare(path, password, parent)?;
    if target.exists() {
        fs::remove_dir(target).map_err(storage)?;
    }
    fs::rename(&restored.directory, target).map_err(storage)?;
    fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(storage)?;
    Ok(json!({"restored":true,"projects":restored.manifest["projects"],"target":target}))
}

fn journal_path(server_directory: &Path) -> PathBuf {
    server_directory
        .parent()
        .unwrap_or(Path::new("."))
        .join("restore.pending.json")
}

pub fn request_restore(projects: &Projects, id: &str, password: &str) -> Result<Value> {
    let _restore = projects
        .restore_lock
        .try_lock()
        .map_err(|_| Error::new("conflict", "A restoration is already being prepared"))?;
    let server = projects
        .server
        .as_ref()
        .ok_or_else(|| Error::new("configuration", "Server setup is required"))?;
    let pending = journal_path(server.directory());
    if pending.exists() {
        return Err(Error::new(
            "conflict",
            "A server restoration is already pending",
        ));
    }
    let mut restored = {
        let _maintenance = projects
            .maintenance
            .try_lock()
            .map_err(|_| Error::new("conflict", "Project maintenance is running"))?;
        prepare(&directory(projects, id)?, password, &root(projects)?)?
    };
    restored.directory = fs::canonicalize(&restored.directory).map_err(storage)?;
    if restored.manifest["server"] != server.status()?["instance"] {
        return Err(Error::new(
            "conflict",
            "Use offline recovery to restore a different server identity",
        ));
    }
    // Preserve a recoverable current generation before an explicitly requested rewind.
    let before = create(projects)?;
    let (programs, data) = projects.roots()?;
    let targets = [
        fs::canonicalize(programs).map_err(storage)?,
        fs::canonicalize(data).map_err(storage)?,
        fs::canonicalize(server.directory()).map_err(storage)?,
    ];
    for (index, target) in targets.iter().enumerate() {
        if targets.iter().enumerate().any(|(other, path)| {
            index != other && (target.starts_with(path) || path.starts_with(target))
        }) {
            return Err(Error::new(
                "configuration",
                "Online recovery requires separate project, database, and server directories",
            ));
        }
    }
    let mut roots = Vec::new();
    for (target, name) in targets.iter().zip(["projects", "databases", "server"]) {
        let stage = restored.directory.join(name);
        let rollback = target.with_file_name(format!(
            ".{}-before-restore-{}",
            target.file_name().unwrap().to_string_lossy(),
            uuid::Uuid::new_v4()
        ));
        roots.push(json!({"target":target,"stage":stage,"rollback":rollback}));
    }
    let document = json!({"version":1,"phase":"prepared","request":uuid::Uuid::new_v4().to_string(),"id":id,"before_backup":before["id"],"staging":restored.directory,"roots":roots});
    crate::server_state::write_private(&pending, &serde_json::to_vec(&document)?)?;
    restored.cleanup = false;
    projects.restart.notify_one();
    Ok(json!({"restoring":true,"request":document["request"],"before_backup":before["id"]}))
}

pub fn freeze_for_restore(projects: &Projects) -> Result<()> {
    let server = projects
        .server
        .as_ref()
        .ok_or_else(|| Error::new("configuration", "Server state is unavailable"))?;
    let path = journal_path(server.directory());
    if !path.exists() {
        return Ok(());
    }
    let mut document: Value = serde_json::from_slice(&fs::read(&path).map_err(storage)?)?;
    let stage = PathBuf::from(document["staging"].as_str().ok_or_else(invalid)?);
    let _maintenance = projects.maintenance.lock().map_err(storage)?;
    projects
        .gate
        .stopped
        .store(true, std::sync::atomic::Ordering::Release);
    let _activity = projects.gate.lock.write().map_err(storage)?;
    let active = projects.active();
    let mut guards = active
        .iter()
        .map(|(_, runtime)| runtime.lock().map_err(storage))
        .collect::<Result<Vec<_>>>()?;
    for ((name, _), runtime) in active.iter().zip(&guards) {
        let destination = stage.join("databases").join(format!("{name}.sqlite"));
        if !destination.exists() {
            // A later-created project becomes inactive after the rewind, but its
            // database is retained and its committed identities cannot be reused.
            runtime.snapshot_to(&destination)?;
        } else {
            let database = rusqlite::Connection::open(&destination)?;
            let mut sequences = runtime
                .db
                .prepare("SELECT entity,value FROM _flow_id_sequences")?;
            for sequence in sequences.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })? {
                let (entity, value) = sequence?;
                database.execute("INSERT INTO _flow_id_sequences(entity,value) VALUES(?1,?2) ON CONFLICT(entity) DO UPDATE SET value=MAX(value,excluded.value)",rusqlite::params![entity,value])?;
            }
        }
    }
    for runtime in &mut guards {
        runtime.suspend()?;
    }
    document["phase"] = json!("activate");
    crate::server_state::write_private(&path, &serde_json::to_vec(&document)?)?;
    Ok(())
}

fn synchronize_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(storage)?;
    }
    Ok(())
}

pub fn activate_pending(server_directory: &Path) -> Result<bool> {
    let path = journal_path(server_directory);
    if !path.exists() {
        return Ok(false);
    }
    if fs::metadata(&path).map_err(storage)?.len() > 65536 {
        return Err(invalid());
    }
    let mut document: Value = serde_json::from_slice(&fs::read(&path).map_err(storage)?)?;
    if document["version"] != 1
        || document["roots"]
            .as_array()
            .is_none_or(|roots| roots.len() != 3)
    {
        return Err(invalid());
    }
    if document["phase"] == "prepared" {
        // A crash before the write boundary keeps the previous working data.
        if let Some(stage) = document["staging"].as_str() {
            let _ = fs::remove_dir_all(stage);
        }
        fs::remove_file(&path).map_err(storage)?;
        synchronize_parent(&path)?;
        return Ok(false);
    }
    let roots = document["roots"].as_array().ok_or_else(invalid)?.clone();
    let activate = (|| -> Result<()> {
        if document["phase"] == "rollback" {
            return Err(Error::new(
                "storage",
                "Completing interrupted restoration rollback",
            ));
        }
        if document["phase"] != "activate" {
            return Err(invalid());
        }
        for root in &roots {
            let target = Path::new(root["target"].as_str().ok_or_else(invalid)?);
            let stage = Path::new(root["stage"].as_str().ok_or_else(invalid)?);
            let rollback = Path::new(root["rollback"].as_str().ok_or_else(invalid)?);
            if stage.exists() {
                if target.exists() && !rollback.exists() {
                    fs::rename(target, rollback).map_err(storage)?;
                    synchronize_parent(target)?;
                }
                if !target.exists() {
                    fs::rename(stage, target).map_err(storage)?;
                    synchronize_parent(target)?;
                } else {
                    return Err(invalid());
                }
            } else if !target.exists() || !rollback.exists() {
                return Err(invalid());
            }
        }
        Ok(())
    })();
    if let Err(error) = activate {
        document["phase"] = json!("rollback");
        crate::server_state::write_private(&path, &serde_json::to_vec(&document)?)?;
        for root in roots.iter().rev() {
            let target = Path::new(root["target"].as_str().ok_or_else(invalid)?);
            let stage = Path::new(root["stage"].as_str().ok_or_else(invalid)?);
            let rollback = Path::new(root["rollback"].as_str().ok_or_else(invalid)?);
            if rollback.exists() {
                if target.exists() {
                    fs::rename(target, stage).map_err(storage)?;
                    synchronize_parent(target)?;
                }
                fs::rename(rollback, target).map_err(storage)?;
                synchronize_parent(target)?;
            }
        }
        if let Some(stage) = document["staging"].as_str() {
            let _ = fs::remove_dir_all(stage);
        }
        fs::remove_file(&path).map_err(storage)?;
        synchronize_parent(&path)?;
        return Err(error);
    }
    let completed = server_directory
        .parent()
        .unwrap_or(Path::new("."))
        .join("restore.last.json");
    crate::server_state::write_private(
        &completed,
        &serde_json::to_vec(
            &json!({"status":"completed","request":document["request"],"id":document["id"],"before_backup":document["before_backup"],"completed_at":chrono::Utc::now().to_rfc3339()}),
        )?,
    )?;
    if let Some(stage) = document["staging"].as_str() {
        let _ = fs::remove_dir_all(stage);
    }
    fs::remove_file(&path).map_err(storage)?;
    synchronize_parent(&path)?;
    Ok(true)
}

pub fn restore_status(projects: &Projects) -> Result<Value> {
    let server = projects
        .server
        .as_ref()
        .ok_or_else(|| Error::new("configuration", "Server state is unavailable"))?;
    let parent = server.directory().parent().unwrap_or(Path::new("."));
    let last = parent.join("restore.last.json");
    if !last.exists() {
        return Ok(json!({"status":"none"}));
    }
    if fs::metadata(&last).map_err(storage)?.len() > 65536 {
        return Err(invalid());
    }
    Ok(serde_json::from_slice(&fs::read(last).map_err(storage)?)?)
}
