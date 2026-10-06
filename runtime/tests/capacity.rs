use flow_runtime::{
    capacity, engine::Config, observability::Observability, projects::Projects, resources,
    server_state::ServerState,
};
use std::{fs, sync::Arc};

#[test]
fn passive_capacity_inventory_preserves_data_and_settings_and_reports_missing_facts() {
    let root = std::env::temp_dir().join(format!("flow-capacity-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("projects")).unwrap();
    let server = Arc::new(ServerState::open(&root.join("server")).unwrap());
    let before = server.status().unwrap();
    let observer = Observability::disk(root.join("logs")).unwrap();
    let projects = Projects::load_with_server(
        &root.join("projects"),
        &root.join("data"),
        Config::default(),
        observer.clone(),
        Some(server.clone()),
    )
    .unwrap();
    let inventory = capacity::inventory(&projects).unwrap();
    assert_eq!(inventory["server"], before["instance"]);
    assert_eq!(inventory["active_calibration"], false);
    assert_eq!(inventory["databases_bytes"], 0);
    assert_eq!(server.status().unwrap(), before);
    #[cfg(unix)]
    {
        assert!(
            inventory["disk"]["logs"]["available_bytes"]
                .as_u64()
                .unwrap()
                > 0
        );
        let disk = resources::disk_space(&root.join("not-created-yet/deep")).unwrap();
        assert!(disk["available_bytes"].as_u64().unwrap() <= disk["total_bytes"].as_u64().unwrap());
    }
    assert_eq!(
        capacity::recommendations(None, 0, 0)["log_target_bytes"],
        serde_json::Value::Null
    );
    let low = capacity::recommendations(Some(10 * 1048576), 0, 0);
    assert_eq!(low["log_target_bytes"], 1048576);
    let growing = capacity::recommendations(Some(20 * 1073741824), 0, 15 * 1073741824);
    assert_eq!(growing["log_target_bytes"], 1048576);
    let healthy = capacity::recommendations(Some(100 * 1073741824), 0, 0);
    assert_eq!(healthy["log_target_bytes"], 1073741824);
    assert_eq!(healthy["log_chunk_bytes"], 50 * 1048576);
    observer.close();
    drop(projects);
    drop(server);
    fs::remove_dir_all(root).unwrap();
}
