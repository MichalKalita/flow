//! Reports host CPU, memory, and disk, and suggests log limits from that inventory.
//! Does not run a load test or change settings by itself.

use crate::{Result, projects::Projects, resources};
use serde_json::{Value, json};
const MIB: u64 = 1048576;
const GIB: u64 = 1024 * MIB;

pub fn recommendations(available: Option<u64>, local_logs: u64, databases: u64) -> Value {
    let reserve = (2 * GIB).max(databases.saturating_mul(2));
    let target = available.map(|free| {
        let spendable = free.saturating_add(local_logs).saturating_sub(reserve);
        (spendable / 10 / MIB * MIB).clamp(MIB, GIB)
    });
    let chunk = target.map(|target| (target / 20 / MIB * MIB).clamp(MIB, 50 * MIB));
    json!({"log_target_bytes":target,"log_chunk_bytes":chunk,"reserve_bytes":reserve,"basis":"Keep at least 2 GiB or two current database copies free; use at most one tenth of the remaining log-volume capacity, capped at 1 GiB.","uncertainty":"Passive storage guidance. Future workload growth, backups on other volumes, and external filesystem quotas may change available capacity."})
}

pub fn inventory(projects: &Projects) -> Result<Value> {
    let (_, data) = projects.roots()?;
    let server = projects.server.as_ref().ok_or_else(|| {
        crate::Error::new("configuration", "Persistent server settings are required")
    })?;
    let settings = server.status()?;
    let mut database_bytes = 0u64;
    let mut complete = true;
    for (_, runtime) in projects.active() {
        let report = runtime
            .lock()
            .map_err(|_| crate::Error::new("internal", "Runtime lock failed"))?
            .storage_resources();
        if let Some(bytes) = report["sqlite"]["total_disk_bytes"].as_u64() {
            database_bytes = database_bytes.saturating_add(bytes);
        } else {
            complete = false;
        }
    }
    let (logs, log_directory) = projects.system.log_source();
    drop(logs);
    let log_directory = log_directory.as_deref().unwrap_or(data);
    let log_disk = resources::disk_space(log_directory);
    let data_disk = resources::disk_space(data);
    let backup_directory = server
        .directory()
        .parent()
        .unwrap_or(server.directory())
        .join("backups");
    let backup_disk = resources::disk_space(&backup_directory);
    let log_usage = projects.system.resources();
    let available = log_disk
        .as_ref()
        .and_then(|disk| disk["available_bytes"].as_u64());
    let recommended = recommendations(
        available,
        log_usage["log_disk_bytes"].as_u64().unwrap_or(0),
        database_bytes,
    );
    let mut notices = Vec::new();
    if available.is_none() {
        notices.push(
            "Available disk space could not be measured. Keep your existing storage settings.",
        );
    }
    if available.is_some_and(|free| free < recommended["reserve_bytes"].as_u64().unwrap()) {
        notices.push("Disk headroom is low. Move a backup off this disk or add space before restoring or migrating applications.");
    }
    let memory = resources::host_memory();
    if memory["limit_bytes"]
        .as_u64()
        .is_some_and(|bytes| bytes < 2 * GIB)
    {
        notices.push("This host is below the 2 GiB production memory baseline.");
    }
    if !complete {
        notices.push("Some application storage measurements are unavailable.");
    }
    let overrides = [
        "FLOW_TOKIO_WORKERS",
        "FLOW_TOKIO_BLOCKING",
        "FLOW_HTTP_ADMISSION",
    ]
    .into_iter()
    .filter_map(|name| std::env::var(name).ok().map(|value| (name, value)))
    .collect::<std::collections::BTreeMap<_, _>>();
    Ok(
        json!({"server":settings["instance"],"server_name":settings["name"],"measured_at":chrono::Utc::now().to_rfc3339(),"cpus":resources::available_parallelism(),"memory":memory,"databases_bytes":database_bytes,"storage_complete":complete,"disk":{"applications":data_disk,"logs":log_disk,"backups":backup_disk},"current":{"log_target_bytes":settings["log_target_bytes"],"log_chunk_bytes":settings["log_chunk_bytes"],"workers":resources::tokio_worker_threads(),"http_admission":resources::http_admission(),"overrides":overrides},"recommendations":recommended,"notices":notices,"active_calibration":false,"note":"No synthetic load was run. Storage recommendations do not estimate application throughput or promise capacity."}),
    )
}
