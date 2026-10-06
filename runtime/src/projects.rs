use crate::{
    Error, Result,
    engine::{Config, Runtime},
    observability::Observability,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    time::SystemTime,
};
pub type SharedRuntime = Arc<Mutex<Runtime>>;
#[derive(Clone)]
struct Entry {
    runtime: Option<SharedRuntime>,
    stamp: Option<(SystemTime, u64, SystemTime, u64)>,
    error: Option<String>,
    generation: u64,
}
pub struct Projects {
    entries: RwLock<BTreeMap<String, Entry>>,
    root: Option<PathBuf>,
    data: PathBuf,
    config: Config,
    pub system: Observability,
    pub server: Option<Arc<crate::server_state::ServerState>>,
    pub(crate) maintenance: Mutex<()>,
    pub public_address: RwLock<Option<std::net::SocketAddr>>,
    pub restart: tokio::sync::Notify,
    pub shutdown: tokio::sync::Notify,
    pub gate: Arc<crate::engine::RuntimeGate>,
    pub(crate) restore_lock: Mutex<()>,
}
fn io(error: std::io::Error) -> Error {
    Error::new("storage", error.to_string())
}
pub(crate) fn name_valid(name: &str) -> bool {
    !["all", "system"].contains(&name)
        && !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
impl Projects {
    pub fn single(runtime: SharedRuntime) -> Arc<Self> {
        Self::single_with_server(runtime, None)
    }
    pub fn single_with_server(
        runtime: SharedRuntime,
        server: Option<Arc<crate::server_state::ServerState>>,
    ) -> Arc<Self> {
        let system = runtime.lock().unwrap().observability.clone();
        let gate = runtime.lock().unwrap().gate.clone().unwrap_or_default();
        runtime.lock().unwrap().gate = Some(gate.clone());
        Arc::new(Self {
            entries: RwLock::new(BTreeMap::from([(
                "application".into(),
                Entry {
                    runtime: Some(runtime),
                    stamp: None,
                    error: None,
                    generation: 1,
                },
            )])),
            root: None,
            data: PathBuf::new(),
            config: Config::default(),
            system,
            server,
            maintenance: Mutex::new(()),
            public_address: RwLock::new(None),
            restart: tokio::sync::Notify::new(),
            shutdown: tokio::sync::Notify::new(),
            gate,
            restore_lock: Mutex::new(()),
        })
    }
    pub fn load(
        root: &Path,
        data: &Path,
        config: Config,
        system: Observability,
    ) -> Result<Arc<Self>> {
        Self::load_with_server(root, data, config, system, None)
    }
    pub fn load_with_server(
        root: &Path,
        data: &Path,
        config: Config,
        system: Observability,
        server: Option<Arc<crate::server_state::ServerState>>,
    ) -> Result<Arc<Self>> {
        std::fs::create_dir_all(data).map_err(io)?;
        let gate = Arc::new(crate::engine::RuntimeGate::default());
        let registry = Arc::new(Self {
            entries: RwLock::new(BTreeMap::new()),
            root: Some(root.into()),
            data: data.into(),
            config,
            system,
            server,
            maintenance: Mutex::new(()),
            public_address: RwLock::new(None),
            restart: tokio::sync::Notify::new(),
            shutdown: tokio::sync::Notify::new(),
            gate,
            restore_lock: Mutex::new(()),
        });
        registry.scan()?;
        Ok(registry)
    }
    pub fn get(&self, name: &str) -> Option<SharedRuntime> {
        self.entries
            .read()
            .unwrap()
            .get(name)
            .and_then(|entry| entry.runtime.clone())
    }
    pub fn active(&self) -> Vec<(String, SharedRuntime)> {
        self.entries
            .read()
            .unwrap()
            .iter()
            .filter_map(|(name, entry)| entry.runtime.clone().map(|r| (name.clone(), r)))
            .collect()
    }
    pub fn default_name(&self) -> Option<String> {
        if self.root.is_none() {
            Some("application".into())
        } else {
            None
        }
    }
    pub fn list(&self) -> Value {
        json!(self.entries.read().unwrap().iter().map(|(name,entry)|json!({"name":name,"active":entry.runtime.is_some(),"status":if entry.error.is_some() {if entry.runtime.is_some(){"stale"}else{"failed"}}else{"running"},"error":entry.error,"generation":entry.generation,"base_path":format!("/{name}")})).collect::<Vec<_>>())
    }
    pub fn scan(&self) -> Result<()> {
        let _maintenance = self
            .maintenance
            .lock()
            .map_err(|_| Error::new("internal", "Project maintenance lock failed"))?;
        self.scan_inner()
    }
    pub(crate) fn scan_inner(&self) -> Result<()> {
        let Some(root) = &self.root else {
            return Ok(());
        };
        let mut folders = std::fs::read_dir(root)
            .map_err(io)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(io)?;
        folders.sort_by_key(|f| f.file_name());
        let mut found = BTreeSet::new();
        let mut count = 0;
        for folder in folders {
            if !folder.file_type().map_err(io)?.is_dir() {
                continue;
            }
            let name = folder.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if !name_valid(&name) {
                let path = root.join(format!("{name}-error.txt"));
                let message = "Project names must contain 1–64 ASCII letters, digits, underscores or hyphens; all and system are reserved\n";
                if std::fs::read_to_string(&path).ok().as_deref() != Some(message) {
                    std::fs::write(path, message).map_err(io)?;
                }
                continue;
            }
            count += 1;
            if count > 32 {
                let path = root.join(format!("{name}-error.txt"));
                let message = "At most 32 projects can be loaded\n";
                if std::fs::read_to_string(&path).ok().as_deref() != Some(message) {
                    std::fs::write(path, message).map_err(io)?;
                }
                continue;
            }
            found.insert(name.clone());
            let source_path = folder.path().join("application.flow");
            let manifest = folder.path().join("project.json");
            let metadata = std::fs::metadata(&source_path);
            let settings = std::fs::metadata(&manifest).ok();
            let stamp = metadata.as_ref().ok().and_then(|m| {
                m.modified().ok().map(|t| {
                    (
                        t,
                        m.len(),
                        settings
                            .as_ref()
                            .and_then(|m| m.modified().ok())
                            .unwrap_or(SystemTime::UNIX_EPOCH),
                        settings.as_ref().map(|m| m.len()).unwrap_or(0),
                    )
                })
            });
            let previous = self.entries.read().unwrap().get(&name).cloned();
            if previous.as_ref().is_some_and(|entry| entry.stamp == stamp) {
                continue;
            }
            let result = (|| -> Result<SharedRuntime> {
                if metadata
                    .as_ref()
                    .map_err(|e| Error::new("storage", e.to_string()))?
                    .len()
                    > 4 * 1024 * 1024
                {
                    return Err(Error::new("limit", "Program exceeds 4 MiB"));
                }
                let source = std::fs::read_to_string(&source_path).map_err(io)?;
                let settings_value = if manifest.exists() {
                    if settings.as_ref().is_some_and(|m| m.len() > 65536) {
                        return Err(Error::new("limit", "Manifest exceeds 64 KiB"));
                    }
                    serde_json::from_str::<Value>(&std::fs::read_to_string(&manifest).map_err(io)?)?
                } else {
                    json!({})
                };
                let config = self.project_config(&name, &source, &settings_value)?;
                let frontend = settings_value
                    .get("frontend")
                    .map(|value| {
                        crate::frontend::validate(
                            &crate::program::Program::compile(&source)?,
                            value,
                        )
                    })
                    .transpose()?;
                let cache = json!({"source":source,"manifest":settings_value});
                let cache_path = self.data.join(format!("{name}-active.json"));
                let save_cache = || -> Result<()> {
                    let temporary = cache_path.with_extension("tmp");
                    std::fs::write(&temporary, serde_json::to_vec(&cache)?).map_err(io)?;
                    std::fs::rename(temporary, &cache_path).map_err(io)?;
                    Ok(())
                };
                if let Some(runtime) = previous.as_ref().and_then(|e| e.runtime.clone()) {
                    {
                        let mut runtime = runtime.lock().unwrap();
                        runtime.reload(&source, config)?;
                        runtime.gate = Some(self.gate.clone());
                        runtime.frontend = frontend.clone();
                    }
                    save_cache()?;
                    return Ok(runtime);
                }
                let database = self.data.join(format!("{name}.sqlite"));
                if database.exists() && cache_path.exists() {
                    // A crash between the SQLite commit and cache rename can leave an
                    // older cache. Prefer an exact current-source match before replaying it.
                    if let Ok(mut runtime) = Runtime::open(
                        &source,
                        database
                            .to_str()
                            .ok_or_else(|| Error::new("configuration", "Invalid database path"))?,
                        config.clone(),
                    ) {
                        runtime.gate = Some(self.gate.clone());
                        runtime.frontend = frontend.clone();
                        runtime.observability = Observability::scoped(
                            self.data.join(format!("{name}-observability")),
                            self.system.clone(),
                            &name,
                        )
                        .map_err(io)?;
                        save_cache()?;
                        return Ok(Arc::new(Mutex::new(runtime)));
                    }
                    if std::fs::metadata(&cache_path).map_err(io)?.len() > 5 * 1024 * 1024 {
                        return Err(Error::new("limit", "Active program cache exceeds 5 MiB"));
                    }
                    let cached: Value =
                        serde_json::from_str(&std::fs::read_to_string(&cache_path).map_err(io)?)?;
                    let active = cached["source"].as_str().ok_or_else(|| {
                        Error::new("configuration", "Invalid active program cache")
                    })?;
                    let active_config = self.project_config(&name, active, &cached["manifest"])?;
                    let mut runtime = Runtime::open(
                        active,
                        database
                            .to_str()
                            .ok_or_else(|| Error::new("configuration", "Invalid database path"))?,
                        active_config,
                    )?;
                    runtime.reload(&source, config)?;
                    runtime.gate = Some(self.gate.clone());
                    runtime.frontend = frontend.clone();
                    runtime.observability = Observability::scoped(
                        self.data.join(format!("{name}-observability")),
                        self.system.clone(),
                        &name,
                    )
                    .map_err(io)?;
                    save_cache()?;
                    return Ok(Arc::new(Mutex::new(runtime)));
                }
                let mut runtime = Runtime::open(
                    &source,
                    database
                        .to_str()
                        .ok_or_else(|| Error::new("configuration", "Invalid database path"))?,
                    config,
                )?;
                runtime.gate = Some(self.gate.clone());
                runtime.frontend = frontend.clone();
                runtime.observability = Observability::scoped(
                    self.data.join(format!("{name}-observability")),
                    self.system.clone(),
                    &name,
                )
                .map_err(io)?;
                save_cache()?;
                Ok(Arc::new(Mutex::new(runtime)))
            })();
            let error_path = root.join(format!("{name}-error.txt"));
            let (runtime, error, generation) = match result {
                Ok(runtime) => {
                    if error_path.exists() {
                        std::fs::remove_file(&error_path).map_err(io)?;
                    }
                    self.system
                        .log(json!({"kind":"project","name":name,"action":"loaded"}));
                    (
                        Some(runtime),
                        None,
                        previous.as_ref().map(|p| p.generation + 1).unwrap_or(1),
                    )
                }
                Err(error) => {
                    let message = error.to_string();
                    std::fs::write(&error_path, format!("{message}\n")).map_err(io)?;
                    self.system
                        .log(json!({"kind":"project","name":name,"success":false,"error":message}));
                    (
                        previous
                            .as_ref()
                            .and_then(|e| e.runtime.clone())
                            .or_else(|| {
                                let cache_path = self.data.join(format!("{name}-active.json"));
                                if std::fs::metadata(&cache_path).ok()?.len() > 5 * 1024 * 1024 {
                                    return None;
                                }
                                let cache: Value = serde_json::from_str(
                                    &std::fs::read_to_string(cache_path).ok()?,
                                )
                                .ok()?;
                                let source = cache["source"].as_str()?;
                                let config = self
                                    .project_config(&name, source, &cache["manifest"])
                                    .ok()?;
                                let mut runtime = Runtime::open(
                                    source,
                                    self.data.join(format!("{name}.sqlite")).to_str()?,
                                    config,
                                )
                                .ok()?;
                                runtime.gate = Some(self.gate.clone());
                                runtime.frontend = cache["manifest"]
                                    .get("frontend")
                                    .map(|value| crate::frontend::validate(&runtime.program, value))
                                    .transpose()
                                    .ok()?;
                                runtime.observability = Observability::scoped(
                                    self.data.join(format!("{name}-observability")),
                                    self.system.clone(),
                                    &name,
                                )
                                .ok()?;
                                Some(Arc::new(Mutex::new(runtime)))
                            }),
                        Some(message),
                        previous.as_ref().map(|p| p.generation).unwrap_or(0),
                    )
                }
            };
            self.entries.write().unwrap().insert(
                name,
                Entry {
                    runtime,
                    stamp,
                    error,
                    generation,
                },
            );
        }
        self.entries
            .write()
            .unwrap()
            .retain(|name, _| found.contains(name));
        Ok(())
    }
    pub fn set_project_secret(&self, project: &str, name: &str, value: &str) -> Result<()> {
        let _maintenance = self
            .maintenance
            .try_lock()
            .map_err(|_| Error::new("conflict", "Project maintenance is already running"))?;
        let server = self
            .server
            .as_ref()
            .ok_or_else(|| Error::new("configuration", "Server secret storage is unavailable"))?;
        if !name_valid(project) {
            return Err(Error::new("invalid_input", "Invalid project name"));
        }
        let Some(runtime) = self.get(project) else {
            return server.set_secret(project, name, value);
        };
        let mut runtime = runtime
            .lock()
            .map_err(|_| Error::new("internal", "Runtime lock failed"))?;
        runtime.require_available()?;
        let mut config = runtime.configuration();
        if self.default_name().is_none() {
            let cache: Value = serde_json::from_slice(
                &std::fs::read(self.data.join(format!("{project}-active.json"))).map_err(io)?,
            )?;
            for (section, jwt) in [("jwt_secrets", true), ("event_credentials", false)] {
                if let Some(bindings) = cache["manifest"][section].as_object() {
                    for (alias, binding) in bindings {
                        if binding["secret"].as_str() == Some(name) {
                            if jwt {
                                config
                                    .jwt_keys
                                    .insert(alias.clone(), value.as_bytes().to_vec());
                            } else {
                                config
                                    .event_credentials
                                    .insert(alias.clone(), format!("ApiKey {value}"));
                            }
                        }
                    }
                }
            }
        }
        if let Err(error) = runtime.validate_configuration(&config) {
            if let Ok((root, _)) = self.roots() {
                std::fs::write(
                    root.join(format!("{project}-error.txt")),
                    format!("Secret configuration: {error}\n"),
                )
                .map_err(io)?;
            }
            return Err(error);
        }
        // Holding the active runtime lock makes persistence and activation one
        // observable boundary. A crash after persistence reopens the new key.
        server.set_secret(project, name, value)?;
        runtime.activate_configuration(config);
        if let Ok((root, _)) = self.roots() {
            let error_path = root.join(format!("{project}-error.txt"));
            if std::fs::read_to_string(&error_path)
                .is_ok_and(|error| error.starts_with("Secret configuration:"))
            {
                std::fs::remove_file(error_path).map_err(io)?;
            }
        }
        Ok(())
    }
    fn project_config(&self, name: &str, source: &str, manifest: &Value) -> Result<Config> {
        let program = crate::program::Program::compile(source)?;
        let mut config = self.config.clone();
        config.jwt_keys.retain(|alias, _| {
            program
                .auth
                .iter()
                .any(|a| a.alias == *alias && a.mode == "jwt")
        });
        config.base_path = format!("/{name}");
        let fields = manifest
            .as_object()
            .ok_or_else(|| Error::new("configuration", "Manifest must be an object"))?;
        if fields.keys().any(|key| {
            ![
                "jwt_secrets",
                "event_credentials",
                "frontend",
                "catalog",
                "test_seeds",
            ]
            .contains(&key.as_str())
        }) {
            return Err(Error::new("configuration", "Unknown manifest setting"));
        }
        config.test_seeds = match manifest.get("test_seeds") {
            None => false,
            Some(value) => value
                .as_bool()
                .ok_or_else(|| Error::new("configuration", "test_seeds must be a boolean"))?,
        };
        if let Some(catalog) = manifest.get("catalog")
            && (!catalog.is_object()
                || catalog["template"] != "contacts"
                || catalog["version"] != 1
                || !catalog["request"]
                    .as_str()
                    .is_some_and(|request| uuid::Uuid::parse_str(request).is_ok()))
        {
            return Err(Error::new("configuration", "Invalid catalog metadata"));
        }
        for (section, mode) in [("jwt_secrets", true), ("event_credentials", false)] {
            if let Some(values) = manifest.get(section) {
                for (alias, env) in values.as_object().ok_or_else(|| {
                    Error::new("configuration", "Manifest settings must be objects")
                })? {
                    let secret = if let Some(variable) = env.as_str() {
                        std::env::var(variable).map_err(|_| {
                            Error::new(
                                "configuration",
                                format!("Missing environment variable {variable}"),
                            )
                        })?
                    } else if let Some(reference) = env
                        .as_object()
                        .filter(|fields| fields.len() == 1)
                        .and_then(|fields| fields.get("secret"))
                        .and_then(Value::as_str)
                    {
                        self.server
                            .as_ref()
                            .ok_or_else(|| {
                                Error::new("configuration", "Server secret storage is unavailable")
                            })?
                            .secret(name, reference)?
                            .ok_or_else(|| {
                                Error::new("configuration", "Required project secret is missing")
                            })?
                    } else {
                        return Err(Error::new(
                            "configuration",
                            "Secret settings require an environment name or a secret reference",
                        ));
                    };
                    if mode {
                        config.jwt_keys.insert(alias.clone(), secret.into_bytes());
                    } else {
                        config
                            .event_credentials
                            .insert(alias.clone(), format!("ApiKey {secret}"));
                    }
                }
            }
        }
        Ok(config)
    }
    pub fn roots(&self) -> Result<(&Path, &Path)> {
        Ok((
            self.root.as_deref().ok_or_else(|| {
                Error::new("configuration", "This operation requires hosted projects")
            })?,
            &self.data,
        ))
    }
    pub fn sample(&self, sample: Value) {
        self.system.sample(sample.clone());
        for (_, runtime) in self.active() {
            runtime.lock().unwrap().observability.sample(sample.clone());
        }
    }
    pub fn flush(&self) -> std::io::Result<()> {
        for (_, runtime) in self.active() {
            runtime.lock().unwrap().observability.flush()?;
        }
        self.system.flush()
    }
}
