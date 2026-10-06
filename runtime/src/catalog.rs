use crate::{
    Error, Result,
    engine::{Config, Runtime},
    projects::Projects,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn storage(_: impl std::fmt::Display) -> Error {
    Error::new("storage", "Application installation storage is unavailable")
}
pub fn index(projects: &Projects) -> Result<Value> {
    let (root, _) = projects.roots()?;
    let mut installed = Vec::new();
    for (name, _) in projects.active() {
        if let Ok(bytes) = fs::read(root.join(&name).join("project.json")) {
            if bytes.len() > 65536 {
                continue;
            }
            if let Ok(manifest) = serde_json::from_slice::<Value>(&bytes)
                && manifest["catalog"]["template"] == "contacts"
            {
                installed.push(
                    json!({"name":name,"title":"Contacts","base_path":format!("/{name}/app")}),
                );
            }
        }
    }
    Ok(
        json!({"templates":[{"id":"contacts","name":"Contacts","description":"Keep names, email addresses, phone numbers, and notes in one place.","version":1}],"installed":installed,"public_port":projects.public_address.read().map_err(storage)?.map(|address|address.port())}),
    )
}
pub fn launch(projects: &Projects, name: &str) -> Result<Value> {
    let (root, _) = projects.roots()?;
    if !crate::projects::name_valid(name) {
        return Err(Error::new("invalid_input", "Invalid application name"));
    }
    let bytes = fs::read(root.join(name).join("project.json")).map_err(storage)?;
    if bytes.len() > 65536 {
        return Err(Error::new(
            "limit",
            "Application manifest exceeds its limit",
        ));
    }
    let manifest: Value = serde_json::from_slice(&bytes)?;
    if manifest["catalog"]["template"] != "contacts" {
        return Err(Error::new(
            "invalid_input",
            "Application is not a catalog instance",
        ));
    }
    let runtime = projects
        .get(name)
        .ok_or_else(|| Error::new("not_found", "Application is unavailable"))?;
    let token = runtime
        .lock()
        .map_err(storage)?
        .issue_admin_jwt("user", "owner", 3600)?;
    Ok(
        json!({"name":name,"base_path":format!("/{name}/app"),"authorization":token["authorization"],"public_port":projects.public_address.read().map_err(storage)?.map(|address|address.port())}),
    )
}
fn metadata() -> Value {
    json!({"title":"Contacts","item_label":"contact","id_input":"contactId","list":"Contacts","create":"CreateContact","update":"UpdateContact","delete":"DeleteContact","fields":[{"name":"name","label":"Name","kind":"text","required":true,"max_bytes":200},{"name":"email","label":"Email","kind":"email","max_bytes":254},{"name":"phone","label":"Phone","kind":"tel","max_bytes":80},{"name":"notes","label":"Notes","kind":"textarea","max_bytes":4096}]})
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
pub fn install(projects: &Projects, template: &str, request: &str) -> Result<Value> {
    if template != "contacts" {
        return Err(Error::new(
            "not_found",
            "This application is not in the catalog",
        ));
    }
    uuid::Uuid::parse_str(request)
        .map_err(|_| Error::new("invalid_input", "Invalid installation request"))?;
    let _maintenance = projects
        .maintenance
        .try_lock()
        .map_err(|_| Error::new("conflict", "Application maintenance is already running"))?;
    let (root, data) = projects.roots()?;
    let server = projects
        .server
        .as_ref()
        .ok_or_else(|| Error::new("configuration", "Server setup is required"))?;
    let entry = if let Some(entry) = server.catalog_entry(request)? {
        entry
    } else {
        if projects
            .list()
            .as_array()
            .is_some_and(|entries| entries.len() >= 32)
        {
            return Err(Error::new("limit", "This server already has 32 projects"));
        }
        let name = (1..=128)
            .map(|n| {
                if n == 1 {
                    "contacts".into()
                } else {
                    format!("contacts-{n}")
                }
            })
            .find(|name| !root.join(name).exists() && !data.join(format!("{name}.sqlite")).exists())
            .ok_or_else(|| Error::new("limit", "No application name is available"))?;
        let mut key = [0u8; 48];
        SystemRandom::new()
            .fill(&mut key)
            .map_err(|_| Error::new("internal", "Key generation failed"))?;
        server.reserve_catalog(request, template, &name, &STANDARD.encode(key))?
    };
    let name = entry["project"]
        .as_str()
        .filter(|name| crate::projects::name_valid(name))
        .ok_or_else(|| Error::new("configuration", "Installation state is invalid"))?;
    if entry["template"] != template {
        return Err(Error::new(
            "conflict",
            "This request belongs to a different application",
        ));
    }
    if root.join(name).exists() {
        let installed: Value = serde_json::from_slice(
            &fs::read(root.join(name).join("project.json")).map_err(storage)?,
        )?;
        if installed["catalog"]["request"] != request {
            return Err(Error::new("conflict", "Application name is already in use"));
        }
        projects.scan_inner()?;
        return launch(projects, name);
    }
    let source = include_str!("catalog/contacts.flow").replace("PROJECT_NAME", name);
    let manifest = json!({"jwt_secrets":{"user":{"secret":"signing"}},"frontend":metadata(),"catalog":{"template":"contacts","version":1,"request":request}});
    let staging = root.join(format!(".catalog-{request}"));
    private_directory(&staging)?;
    let result = (|| -> Result<()> {
        fs::write(staging.join("application.flow"), &source).map_err(storage)?;
        fs::write(staging.join("project.json"), serde_json::to_vec(&manifest)?).map_err(storage)?;
        let mut config = Config {
            base_path: format!("/{name}"),
            ..Config::default()
        };
        config.jwt_keys.insert(
            "user".into(),
            server
                .secret(name, "signing")?
                .ok_or_else(|| Error::new("configuration", "Application signing key is missing"))?
                .into_bytes(),
        );
        let database = data.join(format!("{name}.sqlite"));
        // A committed pending instance is recoverable with the same request/key.
        // No existing instance is replaced and bootstrap audit commits with data.
        let runtime = Runtime::open(
            &source,
            database
                .to_str()
                .ok_or_else(|| Error::new("configuration", "Invalid application path"))?,
            config,
        )?;
        crate::frontend::validate(&runtime.program, &manifest["frontend"])?;
        runtime.validate_storage()?;
        drop(runtime);
        for filename in ["application.flow", "project.json"] {
            fs::File::open(staging.join(filename))
                .and_then(|file| file.sync_all())
                .map_err(storage)?;
        }
        fs::File::open(&staging)
            .and_then(|file| file.sync_all())
            .map_err(storage)?;
        fs::rename(&staging, root.join(name)).map_err(storage)?;
        fs::File::open(root)
            .and_then(|file| file.sync_all())
            .map_err(storage)?;
        projects.scan_inner()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result?;
    launch(projects, name)
}
