//! Persistent server identity, encryption keys, settings, and project secrets.
//! Stores secrets encrypted. Does not log secret values.

use crate::{Error, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ring::{
    aead, pbkdf2,
    rand::{SecureRandom, SystemRandom},
    signature::{Ed25519KeyPair, KeyPair},
};
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_STATE_BYTES: usize = 1024 * 1024;
const PASSWORD_ROUNDS: u32 = 210_000;

pub struct ServerState {
    database: Mutex<Connection>,
    encryption: aead::LessSafeKey,
    directory: PathBuf,
    instance: String,
    public_key: String,
}

fn storage(_: impl std::fmt::Display) -> Error {
    Error::new("storage", "Server state could not be read or written")
}
fn crypto() -> Error {
    Error::new(
        "configuration",
        "Server encryption material is missing or invalid",
    )
}
fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    SystemRandom::new().fill(&mut bytes).map_err(|_| crypto())?;
    Ok(bytes)
}
fn private_permissions(path: &Path, directory: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
        )
        .map_err(storage)?;
    }
    #[cfg(not(unix))]
    let _ = (path, directory);
    Ok(())
}
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(storage)?;
        file.write_all(bytes).map_err(storage)?;
        file.sync_all().map_err(storage)?;
        fs::rename(&temporary, path).map_err(storage)?;
        if let Some(parent) = path.parent() {
            fs::File::open(parent)
                .and_then(|f| f.sync_all())
                .map_err(storage)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
fn aad(revision: i64) -> aead::Aad<Vec<u8>> {
    aead::Aad::from(format!("flow.server.state.v1/{revision}").into_bytes())
}

impl ServerState {
    pub fn open(directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory).map_err(storage)?;
        private_permissions(directory, true)?;
        let path = directory.join("server.sqlite");
        let database = Connection::open(&path)?;
        private_permissions(&path, false)?;
        database.busy_timeout(std::time::Duration::from_secs(5))?;
        // SQLite journal-mode changes can report BUSY without invoking its busy
        // handler during concurrent first opens. Retry that bootstrap boundary.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match database.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS state (id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL, document BLOB NOT NULL); BEGIN IMMEDIATE;") {
                Ok(()) => break,
                Err(rusqlite::Error::SqliteFailure(error, _)) if matches!(error.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) && std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(10)),
                Err(error) => return Err(error.into()),
            }
        }
        let initialized: bool =
            database.query_row("SELECT EXISTS(SELECT 1 FROM state)", [], |r| r.get(0))?;
        let keys_path = directory.join("server.keys");
        let result = (|| {
            let keys: Value = if keys_path.exists() {
                if fs::metadata(&keys_path).map_err(storage)?.len() > 16_384 {
                    return Err(crypto());
                }
                serde_json::from_slice(&fs::read(&keys_path).map_err(storage)?)
                    .map_err(|_| crypto())?
            } else {
                if initialized {
                    return Err(crypto());
                }
                let pair =
                    Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| crypto())?;
                let keys = json!({"version":1,"instance":uuid::Uuid::new_v4().to_string(),"identity_key":STANDARD.encode(pair.as_ref()),"encryption_key":STANDARD.encode(random::<32>()?)});
                write_private(&keys_path, &serde_json::to_vec(&keys)?)?;
                keys
            };
            if keys["version"] != 1 {
                return Err(crypto());
            }
            let instance = keys["instance"].as_str().ok_or_else(crypto)?.to_owned();
            uuid::Uuid::parse_str(&instance).map_err(|_| crypto())?;
            let private = STANDARD
                .decode(keys["identity_key"].as_str().ok_or_else(crypto)?)
                .map_err(|_| crypto())?;
            let pair = Ed25519KeyPair::from_pkcs8(&private).map_err(|_| crypto())?;
            let key = STANDARD
                .decode(keys["encryption_key"].as_str().ok_or_else(crypto)?)
                .map_err(|_| crypto())?;
            let encryption = aead::LessSafeKey::new(
                aead::UnboundKey::new(&aead::AES_256_GCM, &key).map_err(|_| crypto())?,
            );
            Ok((
                instance,
                STANDARD.encode(pair.public_key().as_ref()),
                encryption,
            ))
        })();
        let (instance, public_key, encryption) = match result {
            Ok(value) => value,
            Err(error) => {
                let _ = database.execute_batch("ROLLBACK");
                return Err(error);
            }
        };
        let state = Self {
            database: Mutex::new(database),
            encryption,
            directory: directory.into(),
            instance,
            public_key,
        };
        let result = (|| {
            let database = state.database.lock().map_err(storage)?;
            if initialized {
                state.read_document(&database)?;
            } else {
                let short = &state.instance[..8];
                let available = crate::resources::disk_space(directory)
                    .and_then(|disk| disk["available_bytes"].as_u64());
                let recommended = crate::capacity::recommendations(available, 0, 0);
                let log_target = recommended["log_target_bytes"]
                    .as_u64()
                    .unwrap_or(crate::observability::LOG_DISK_LIMIT_BYTES);
                let log_chunk = recommended["log_chunk_bytes"]
                    .as_u64()
                    .unwrap_or(crate::observability::LOG_CHUNK_BYTES);
                let defaults = json!({"name":format!("flow-{short}"),"setup_complete":false,"admin_token":STANDARD.encode(random::<48>()?),"secrets":{},"variables":{},"backup_keep":7,"log_target_bytes":log_target,"log_chunk_bytes":log_chunk});
                state.write_document(&database, 1, &defaults)?;
            }
            database.execute_batch("COMMIT")?;
            Ok(())
        })();
        if let Err(error) = result {
            if let Ok(database) = state.database.lock() {
                let _ = database.execute_batch("ROLLBACK");
            }
            return Err(error);
        }
        Ok(state)
    }

    fn read_document(&self, database: &Connection) -> Result<(i64, Value)> {
        let (revision, mut encrypted): (i64, Vec<u8>) = database.query_row(
            "SELECT revision,document FROM state WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if encrypted.len() < 28 || encrypted.len() > MAX_STATE_BYTES + 28 || revision < 1 {
            return Err(crypto());
        }
        let nonce: [u8; 12] = encrypted[..12].try_into().map_err(|_| crypto())?;
        let plain = self
            .encryption
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aad(revision),
                &mut encrypted[12..],
            )
            .map_err(|_| crypto())?;
        let document: Value = serde_json::from_slice(plain).map_err(|_| crypto())?;
        if !document.is_object()
            || !document["name"].is_string()
            || !document["admin_token"].is_string()
            || !document["secrets"].is_object()
        {
            return Err(crypto());
        }
        Ok((revision, document))
    }

    fn write_document(&self, database: &Connection, revision: i64, document: &Value) -> Result<()> {
        let mut plain = serde_json::to_vec(document)?;
        if plain.len() > MAX_STATE_BYTES {
            return Err(Error::new("limit", "Server settings exceed 1 MiB"));
        }
        let nonce = random::<12>()?;
        self.encryption
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(nonce),
                aad(revision),
                &mut plain,
            )
            .map_err(|_| crypto())?;
        let mut encrypted = nonce.to_vec();
        encrypted.extend(plain);
        database.execute("INSERT INTO state(id,revision,document) VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,document=excluded.document", rusqlite::params![revision, encrypted])?;
        Ok(())
    }

    fn change(
        &self,
        expected: Option<i64>,
        update: impl FnOnce(&mut Value) -> Result<()>,
    ) -> Result<Value> {
        let database = self.database.lock().map_err(storage)?;
        database.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            let (revision, mut document) = self.read_document(&database)?;
            if expected.is_some_and(|value| value != revision) {
                return Err(Error::new(
                    "conflict",
                    "Settings changed; reload before saving",
                ));
            }
            update(&mut document)?;
            let next = revision.checked_add(1).ok_or_else(crypto)?;
            self.write_document(&database, next, &document)?;
            Ok(document)
        })();
        match result {
            Ok(document) => {
                database.execute_batch("COMMIT")?;
                Ok(document)
            }
            Err(error) => {
                let _ = database.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    pub fn admin_token(&self) -> Result<String> {
        let database = self.database.lock().map_err(storage)?;
        Ok(self.read_document(&database)?.1["admin_token"]
            .as_str()
            .ok_or_else(crypto)?
            .into())
    }

    pub fn status(&self) -> Result<Value> {
        let database = self.database.lock().map_err(storage)?;
        let (revision, document) = self.read_document(&database)?;
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(STANDARD.decode(&self.public_key).map_err(|_| crypto())?)
        );
        Ok(
            json!({"instance":self.instance,"name":document["name"],"public_key":self.public_key,"fingerprint":fingerprint,"setup_complete":document["setup_complete"],"revision":revision,"backup_keep":document["backup_keep"],"log_target_bytes":document["log_target_bytes"].as_u64().unwrap_or(crate::observability::LOG_DISK_LIMIT_BYTES),"log_chunk_bytes":document["log_chunk_bytes"].as_u64().unwrap_or(crate::observability::LOG_CHUNK_BYTES),"defaults":{"tokio_workers":crate::resources::tokio_worker_threads(),"tokio_blocking":crate::resources::tokio_blocking_threads(),"http_admission":crate::resources::http_admission()},"secrets":document["secrets"].as_object().ok_or_else(crypto)?.iter().map(|(name,value)|json!({"name":name,"preview":masked(value.as_str().unwrap_or(""))})).collect::<Vec<_>>()}),
        )
    }

    pub fn update_settings(&self, input: &Value) -> Result<Value> {
        let fields = input
            .as_object()
            .ok_or_else(|| Error::new("invalid_input", "Expected settings object"))?;
        if fields.keys().any(|key| {
            ![
                "revision",
                "name",
                "backup_keep",
                "log_target_bytes",
                "log_chunk_bytes",
            ]
            .contains(&key.as_str())
        }) {
            return Err(Error::new("invalid_input", "Unknown server setting"));
        }
        let revision = fields
            .get("revision")
            .and_then(Value::as_i64)
            .ok_or_else(|| Error::new("invalid_input", "Settings revision is required"))?;
        self.change(Some(revision), |document| {
            let target = input.get("log_target_bytes").unwrap_or(&document["log_target_bytes"]).as_u64().unwrap_or(crate::observability::LOG_DISK_LIMIT_BYTES);
            let chunk = input.get("log_chunk_bytes").unwrap_or(&document["log_chunk_bytes"]).as_u64().unwrap_or(crate::observability::LOG_CHUNK_BYTES);
            if ["log_target_bytes","log_chunk_bytes"].iter().any(|key|input.get(*key).is_some_and(|value|value.as_u64().is_none())) || !(1048576..=107374182400).contains(&target) || !(1048576..=target).contains(&chunk) || target / chunk > 512 {
                return Err(Error::new("invalid_input", "Log chunks must be at least 1 MiB and fit the local target, with at most 512 chunks"));
            }
            document["log_target_bytes"]=json!(target);
            document["log_chunk_bytes"]=json!(chunk);
            if let Some(name) = fields.get("name") {
                validate_name(name.as_str().unwrap_or(""))?;
                document["name"] = name.clone();
            }
            if let Some(keep) = fields.get("backup_keep") {
                if !keep.as_u64().is_some_and(|n| (1..=100).contains(&n)) {
                    return Err(Error::new(
                        "invalid_input",
                        "Keep between 1 and 100 backups",
                    ));
                }
                document["backup_keep"] = keep.clone();
            }
            Ok(())
        })?;
        self.status()
    }

    pub fn enroll(&self, name: &str, password: &str) -> Result<String> {
        validate_name(name)?;
        if !(10..=512).contains(&password.len()) {
            return Err(Error::new(
                "invalid_input",
                "Use a password of 10 to 512 bytes",
            ));
        }
        let salt = random::<32>()?;
        let mut hash = [0; 32];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            NonZeroU32::new(PASSWORD_ROUNDS).unwrap(),
            &salt,
            password.as_bytes(),
            &mut hash,
        );
        let document = self.change(None, |document| {
            if document["setup_complete"] == true {
                return Err(Error::new(
                    "conflict",
                    "This server has already been set up",
                ));
            }
            let keys = fs::read(self.directory.join("server.keys")).map_err(storage)?;
            let recovery = protect_recovery(&keys, password)?;
            write_private(&self.directory.join("recovery.keys"), &recovery)?;
            document["name"] = json!(name);
            document["owner_salt"] = json!(STANDARD.encode(salt));
            document["owner_hash"] = json!(STANDARD.encode(hash));
            document["setup_complete"] = json!(true);
            Ok(())
        })?;
        Ok(document["admin_token"].as_str().ok_or_else(crypto)?.into())
    }

    pub fn login(&self, password: &str) -> Result<String> {
        if password.len() > 512 {
            return Err(Error::new("unauthenticated", "The password is not valid"));
        }
        let document = {
            let database = self.database.lock().map_err(storage)?;
            self.read_document(&database)?.1
        };
        let rejected = || Error::new("unauthenticated", "The password is not valid");
        let salt = STANDARD
            .decode(document["owner_salt"].as_str().ok_or_else(rejected)?)
            .map_err(|_| rejected())?;
        let hash = STANDARD
            .decode(document["owner_hash"].as_str().ok_or_else(rejected)?)
            .map_err(|_| rejected())?;
        pbkdf2::verify(
            pbkdf2::PBKDF2_HMAC_SHA256,
            NonZeroU32::new(PASSWORD_ROUNDS).unwrap(),
            &salt,
            password.as_bytes(),
            &hash,
        )
        .map_err(|_| rejected())?;
        Ok(document["admin_token"].as_str().ok_or_else(crypto)?.into())
    }

    pub fn set_secret(&self, namespace: &str, name: &str, value: &str) -> Result<()> {
        if namespace.is_empty()
            || namespace.len() > 64
            || name.is_empty()
            || name.len() > 64
            || !namespace
                .bytes()
                .chain(name.bytes())
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || value.len() > 65536
        {
            return Err(Error::new("invalid_input", "Invalid secret name or size"));
        }
        self.change(None, |document| {
            document["secrets"][format!("{namespace}/{name}")] = json!(value);
            Ok(())
        })?;
        Ok(())
    }

    pub fn secret(&self, namespace: &str, name: &str) -> Result<Option<String>> {
        let database = self.database.lock().map_err(storage)?;
        let document = self.read_document(&database)?.1;
        Ok(document["secrets"][format!("{namespace}/{name}")]
            .as_str()
            .map(str::to_owned))
    }

    pub fn snapshot(&self, path: &Path) -> Result<()> {
        let database = self.database.lock().map_err(storage)?;
        database.execute(
            "VACUUM INTO ?1",
            [path
                .to_str()
                .ok_or_else(|| Error::new("invalid_input", "Invalid backup path"))?],
        )?;
        private_permissions(path, false)
    }

    pub fn snapshot_with_secrets(
        &self,
        path: &Path,
        secrets: &[(String, String, String)],
    ) -> Result<()> {
        self.snapshot(path)?;
        let database = Connection::open(path)?;
        let (revision, mut document) = self.read_document(&database)?;
        for (namespace, name, value) in secrets {
            document["secrets"][format!("{namespace}/{name}")] = json!(value);
        }
        self.write_document(
            &database,
            revision.checked_add(1).ok_or_else(crypto)?,
            &document,
        )?;
        Ok(())
    }
    pub fn catalog_entry(&self, request: &str) -> Result<Option<Value>> {
        let database = self.database.lock().map_err(storage)?;
        let document = self.read_document(&database)?.1;
        Ok(document["catalog"].get(request).cloned())
    }
    pub fn reserve_catalog(
        &self,
        request: &str,
        template: &str,
        project: &str,
        signing_key: &str,
    ) -> Result<Value> {
        uuid::Uuid::parse_str(request)
            .map_err(|_| Error::new("invalid_input", "Invalid installation request"))?;
        if !crate::projects::name_valid(project) {
            return Err(Error::new("invalid_input", "Invalid project name"));
        }
        let document = self.change(None, |document| {
            if document["catalog"].get(request).is_some() {
                return Ok(());
            }
            if document["catalog"]
                .as_object()
                .is_some_and(|entries| entries.len() >= 128)
            {
                return Err(Error::new("limit", "Installation history is full"));
            }
            document["catalog"][request] = json!({"template":template,"project":project});
            document["secrets"][format!("{project}/signing")] = json!(signing_key);
            Ok(())
        })?;
        Ok(document["catalog"][request].clone())
    }
    pub fn sign_backup_manifest(&self, bytes: &[u8]) -> Result<String> {
        let keys: Value =
            serde_json::from_slice(&fs::read(self.directory.join("server.keys")).map_err(storage)?)
                .map_err(|_| crypto())?;
        let private = STANDARD
            .decode(keys["identity_key"].as_str().ok_or_else(crypto)?)
            .map_err(|_| crypto())?;
        let pair = Ed25519KeyPair::from_pkcs8(&private).map_err(|_| crypto())?;
        let mut message = b"flow.backup.manifest.v1\0".to_vec();
        message.extend_from_slice(bytes);
        Ok(STANDARD.encode(pair.sign(&message).as_ref()))
    }
    pub fn close(&self) -> Result<()> {
        let mut database = self.database.lock().map_err(storage)?;
        *database = Connection::open_in_memory()?;
        Ok(())
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
        return Err(Error::new(
            "invalid_input",
            "Use a server name of 1 to 80 characters",
        ));
    }
    Ok(())
}

fn masked(value: &str) -> String {
    let chars: Vec<_> = value.chars().collect();
    if chars.len() < 16 {
        return "••••••••".into();
    }
    format!(
        "{}••••••••{}",
        chars[..4].iter().collect::<String>(),
        chars[chars.len() - 4..].iter().collect::<String>()
    )
}

fn protect_recovery(keys: &[u8], password: &str) -> Result<Vec<u8>> {
    let salt = random::<32>()?;
    let mut derived = [0; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(PASSWORD_ROUNDS).unwrap(),
        &salt,
        password.as_bytes(),
        &mut derived,
    );
    let encryption = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, &derived).map_err(|_| crypto())?,
    );
    let nonce = random::<12>()?;
    let mut encrypted = keys.to_vec();
    encryption
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(b"flow.server.recovery.v1"),
            &mut encrypted,
        )
        .map_err(|_| crypto())?;
    let mut payload = nonce.to_vec();
    payload.extend(encrypted);
    Ok(serde_json::to_vec(
        &json!({"version":1,"salt":STANDARD.encode(salt),"payload":STANDARD.encode(payload)}),
    )?)
}

pub fn unlock_recovery(package: &[u8], password: &str) -> Result<Vec<u8>> {
    if package.len() > 16384 || password.len() > 512 {
        return Err(crypto());
    }
    let package: Value = serde_json::from_slice(package).map_err(|_| crypto())?;
    if package["version"] != 1 {
        return Err(crypto());
    }
    let salt = STANDARD
        .decode(package["salt"].as_str().ok_or_else(crypto)?)
        .map_err(|_| crypto())?;
    if salt.len() != 32 {
        return Err(crypto());
    }
    let mut derived = [0; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(PASSWORD_ROUNDS).unwrap(),
        &salt,
        password.as_bytes(),
        &mut derived,
    );
    let encryption = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, &derived).map_err(|_| crypto())?,
    );
    let mut payload = STANDARD
        .decode(package["payload"].as_str().ok_or_else(crypto)?)
        .map_err(|_| crypto())?;
    if payload.len() < 28 {
        return Err(crypto());
    }
    let nonce: [u8; 12] = payload[..12].try_into().map_err(|_| crypto())?;
    let decrypted = encryption
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(b"flow.server.recovery.v1"),
            &mut payload[12..],
        )
        .map_err(|_| {
            Error::new(
                "unauthenticated",
                "The backup recovery password is not valid",
            )
        })?;
    Ok(decrypted.to_vec())
}

pub struct RuntimeLease {
    _database: Connection,
}
impl RuntimeLease {
    pub fn acquire(directory: &Path) -> Result<Self> {
        let parent = directory.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(storage)?;
        let path = parent.join("server.lease.sqlite");
        let database = Connection::open(&path)?;
        private_permissions(&path, false)?;
        database
            .execute_batch("PRAGMA busy_timeout=0; BEGIN EXCLUSIVE;")
            .map_err(|_| {
                Error::new(
                    "conflict",
                    "Another Flow process is using this server directory",
                )
            })?;
        Ok(Self {
            _database: database,
        })
    }
}

pub fn verify_backup_manifest(keys: &[u8], bytes: &[u8], signature: &str) -> Result<()> {
    let keys: Value = serde_json::from_slice(keys).map_err(|_| crypto())?;
    let private = STANDARD
        .decode(keys["identity_key"].as_str().ok_or_else(crypto)?)
        .map_err(|_| crypto())?;
    let pair = Ed25519KeyPair::from_pkcs8(&private).map_err(|_| crypto())?;
    let signature = STANDARD.decode(signature).map_err(|_| crypto())?;
    let mut message = b"flow.backup.manifest.v1\0".to_vec();
    message.extend_from_slice(bytes);
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, pair.public_key().as_ref())
        .verify(&message, &signature)
        .map_err(|_| Error::new("invalid_input", "Backup manifest signature is not valid"))
}
