use flow_runtime::{
    engine::{Config, Runtime},
    http, mqtt,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{Duration, timeout},
};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
const USER_KEY: &str = "user-key-long-enough-1234567890123456";
const DEVICE_KEY: &str = "device-key-long-enough-12345678901234";
fn source() -> String {
    let user = format!("{:x}", Sha256::digest(USER_KEY));
    let device = format!("{:x}", Sha256::digest(DEVICE_KEY));
    format!(
        r#"
[type UserID [id User]] [type DeviceID [id Device]] [type AccessID [id Access]]
[type Battery [integer [range 0 100]]]
[entity User [field id UserID] [field hash String [unique]]]
[entity Device [field id DeviceID] [field hash String [unique]] [field access [list Access] [inverse Access.device]] [field status [list Status] [stream Status.device]]]
[entity Access [field id AccessID] [field device Device] [field user User] [field active Bool]]
[stream Status [mqtt "devices/{{device}}/status"] [history [duration "1h"] [maxMessages 2]] [field device Device] [field battery Battery] [field online Bool]]
[seed User [rows [record [id "u1"] [hash "{user}"]]]]
[seed Device [rows [record [id "d1"] [hash "{device}"]] [record [id "d2"] [hash "other"]]]]
[seed Access [rows [record [id "a1"] [device "d1"] [user "u1"] [active true]]]]
[auth [user User [apiKey] [entity [eq User.hash credential.hash]]] [device Device [apiKey] [entity [eq Device.hash credential.hash]]]]
[transport HTTP [auth user]] [transport MQTT [auth device user]] [transport WebSocket [auth user]]
[permissions Device [Status [CREATE [when [eq actor after.device]]]]]
[permissions User [Device [READ [when [any target.access access [and [eq actor access.user] access.active]]]]] [Status [READ [when [can READ target.device]]]] [Access [UPDATE [when [eq actor target.user]]]]]
[type StatusOutput [record [field battery Battery]]]
[type DeviceOutput [record [field id DeviceID] [field status [optional StatusOutput]]]]
[query Latest [input id DeviceID] [output DeviceOutput] [http GET "/devices/{{id}}"] [device [entity $id]] [result [record [id device.id] [status [single [last device.status 1]]]]]]
[query LiveStatus [input id DeviceID] [output StatusOutput] [websocket "/ws" [source Status]] [device [entity $id]] [result [live device.status]]]
[mutate Revoke [input id AccessID] [output Bool] [http POST "/revoke/{{id}}"] [atomic] [access [entity $id]] [changed [set access.active false]] [result true]]
"#
    )
}
fn runtime() -> Runtime {
    Runtime::open(&source(), ":memory:", Config::default()).unwrap()
}
fn auth(key: &str) -> String {
    format!("ApiKey {key}")
}
#[test]
fn publish_auth_retention_and_latest_projection() {
    let mut r = runtime();
    let device = auth(DEVICE_KEY);
    let user = auth(USER_KEY);
    assert!(
        r.publish_topic(
            "devices/d2/status",
            json!({"battery":50,"online":true}),
            Some(&device)
        )
        .is_err()
    );
    assert!(
        r.publish_topic(
            "devices/d1/status",
            json!({"battery":50,"online":true,"receivedAt":"2000-01-01T00:00:00Z"}),
            Some(&device)
        )
        .is_err()
    );
    for value in [50, 40, 30] {
        r.publish_topic(
            "devices/d1/status",
            json!({"battery":value,"online":true}),
            Some(&device),
        )
        .unwrap();
    }
    let latest = r
        .execute("Latest", json!({"id":"d1"}), Some(&user))
        .unwrap();
    assert_eq!(latest["status"]["battery"], 30);
    let (values, trace, keys) = r
        .execute_transport("LiveStatus", json!({"id":"d1"}), Some(&user), "WebSocket")
        .unwrap();
    assert_eq!(values, json!([{"battery":40},{"battery":30}]));
    assert_eq!(keys.len(), 2);
    assert!(trace.iter().all(|s| !s.contains("online")));
    let messages = r.mqtt_messages("devices/+/status", Some(&user)).unwrap();
    assert_eq!(messages.len(), 2);
    r.execute("Revoke", json!({"id":"a1"}), Some(&user))
        .unwrap();
    assert_eq!(r.mqtt_messages("devices/#", Some(&user)).unwrap().len(), 0);
    assert!(
        r.execute_transport("LiveStatus", json!({"id":"d1"}), Some(&user), "WebSocket")
            .is_err()
    );
}
async fn write_packet(socket: &mut TcpStream, header: u8, body: &[u8]) {
    assert!(body.len() < 128);
    socket.write_all(&[header, body.len() as u8]).await.unwrap();
    socket.write_all(body).await.unwrap();
}
async fn read_packet(socket: &mut TcpStream) -> (u8, Vec<u8>) {
    timeout(Duration::from_secs(3), async {
        let header = socket.read_u8().await.unwrap();
        let mut length = 0usize;
        let mut factor = 1;
        loop {
            let byte = socket.read_u8().await.unwrap();
            length += (byte as usize & 127) * factor;
            if byte & 128 == 0 {
                break;
            };
            factor *= 128;
        }
        let mut body = vec![0; length];
        socket.read_exact(&mut body).await.unwrap();
        (header, body)
    })
    .await
    .unwrap()
}
fn string(bytes: &mut Vec<u8>, s: &str) {
    bytes.extend((s.len() as u16).to_be_bytes());
    bytes.extend(s.as_bytes());
}
async fn connect(address: std::net::SocketAddr, alias: &str, key: &str) -> TcpStream {
    let mut socket = TcpStream::connect(address).await.unwrap();
    let mut body = vec![];
    string(&mut body, "MQTT");
    body.extend([4, 0xc2, 0, 30]);
    string(&mut body, "test");
    string(&mut body, alias);
    string(&mut body, key);
    write_packet(&mut socket, 0x10, &body).await;
    assert_eq!(read_packet(&mut socket).await, (0x20, vec![0, 0]));
    socket
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mqtt_and_websocket_share_permissions_and_sqlite() {
    let runtime = Arc::new(Mutex::new(runtime()));
    let mqtt_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mqtt_address = mqtt_listener.local_addr().unwrap();
    let mqtt_runtime = runtime.clone();
    let mqtt_server =
        tokio::spawn(async move { mqtt::serve(mqtt_listener, mqtt_runtime).await.unwrap() });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let http_runtime = runtime.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, http::router_shared(http_runtime))
            .await
            .unwrap()
    });
    let mut request = format!("ws://{address}/ws").into_client_request().unwrap();
    request
        .headers_mut()
        .insert("authorization", auth(USER_KEY).parse().unwrap());
    let (mut websocket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    websocket
        .send(Message::Text(
            json!({"id":"s1","query":"LiveStatus","input":{"id":"d1"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let mut subscriber = connect(mqtt_address, "user", USER_KEY).await;
    let mut body = vec![0, 1];
    string(&mut body, "devices/+/status");
    body.push(0);
    write_packet(&mut subscriber, 0x82, &body).await;
    assert_eq!(read_packet(&mut subscriber).await, (0x90, vec![0, 1, 0]));
    let mut publisher = connect(mqtt_address, "device", DEVICE_KEY).await;
    let mut body = vec![];
    string(&mut body, "devices/d1/status");
    body.extend([0, 2]);
    body.extend(br#"{"battery":12,"online":true}"#);
    // A frame arriving across timer ticks must not lose its partial parser state.
    publisher
        .write_all(&[0x32, body.len() as u8])
        .await
        .unwrap();
    publisher.write_all(&body[..4]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    publisher.write_all(&body[4..]).await.unwrap();
    assert_eq!(read_packet(&mut publisher).await, (0x40, vec![0, 2]));
    let ws = timeout(Duration::from_secs(3), websocket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let json: Value = serde_json::from_str(ws.to_text().unwrap()).unwrap();
    assert_eq!(json, json!({"id":"s1","data":{"battery":12}}));
    let (header, body) = read_packet(&mut subscriber).await;
    assert_eq!(header, 0x30);
    let length = u16::from_be_bytes([body[0], body[1]]) as usize;
    let payload: Value = serde_json::from_slice(&body[2 + length..]).unwrap();
    assert_eq!(payload["battery"], 12);
    runtime
        .lock()
        .unwrap()
        .execute("Revoke", json!({"id":"a1"}), Some(&auth(USER_KEY)))
        .unwrap();
    let ws = timeout(Duration::from_secs(3), websocket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(ws.to_text().unwrap()).unwrap()["error"],
        "not_found"
    );
    let mut body = vec![];
    string(&mut body, "devices/d1/status");
    body.extend([0, 3]);
    body.extend(br#"{"battery":11,"online":true}"#);
    write_packet(&mut publisher, 0x32, &body).await;
    assert_eq!(read_packet(&mut publisher).await, (0x40, vec![0, 3]));
    assert!(
        timeout(Duration::from_millis(250), subscriber.read_u8())
            .await
            .is_err()
    );
    server.abort();
    mqtt_server.abort();
}

const AUTOMATION_KEY: &str = "automation-key-long-enough-123456789";
fn event_source() -> String {
    let hash = format!("{:x}", Sha256::digest(AUTOMATION_KEY));
    source().replacen("[auth ", &format!("[type ServiceID [id Service]] [type AlertID [id Alert]]\n[entity Service [field id ServiceID] [field hash String [unique]]]\n[entity Alert [field id AlertID] [field device Device] [field battery Battery]]\n[seed Service [rows [record [id \"automation\"] [hash \"{hash}\"]]]]\n[permissions Service [Device [READ [when true]]] [Status [READ [when [can READ target.device]]]] [Alert [CREATE [when true]] [READ [when true]]]]\n[type AlertOutput [record [field id AlertID] [field battery Battery]]]\n[mutate LowBattery [output AlertOutput] [on Status [actor [service \"automation\"]]] [when [lt event.battery 20]] [atomic] [alert [create Alert [record [id [new AlertID]] [device event.device] [battery event.battery]]]] [result alert]]\n[query Alerts [output [list AlertOutput [max 10]]] [http GET \"/alerts\"] [result [first [entities Alert] 10]]]\n[auth [service Service [apiKey] [entity [eq Service.hash credential.hash]]] "),1).replace("[transport HTTP [auth user]]", "[transport HTTP [auth user service]]")
}
#[test]
fn event_handlers_use_verified_native_actor_and_transactional_permissions() {
    let source = event_source();
    assert!(Runtime::open(&source, ":memory:", Config::default()).is_err());
    let mut config = Config::default();
    config
        .event_credentials
        .insert("service:automation".into(), auth(AUTOMATION_KEY));
    let mut runtime = Runtime::open(&source, ":memory:", config.clone()).unwrap();
    let device = auth(DEVICE_KEY);
    let service = auth(AUTOMATION_KEY);
    runtime
        .publish_topic(
            "devices/d1/status",
            json!({"battery":30,"online":true}),
            Some(&device),
        )
        .unwrap();
    assert_eq!(
        runtime
            .execute("Alerts", json!({}), Some(&service))
            .unwrap(),
        json!([])
    );
    runtime
        .publish_topic(
            "devices/d1/status",
            json!({"battery":12,"online":true}),
            Some(&device),
        )
        .unwrap();
    assert_eq!(
        runtime
            .execute("Alerts", json!({}), Some(&service))
            .unwrap()[0]["battery"],
        12
    );
    assert_eq!(
        runtime
            .execute("LowBattery", json!({}), Some(&service))
            .unwrap_err()
            .code,
        "not_found"
    );
    let denied = source.replace(
        "[CREATE [when true]] [READ [when true]]",
        "[CREATE [when false]] [READ [when true]]",
    );
    let mut runtime = Runtime::open(&denied, ":memory:", config).unwrap();
    assert_eq!(
        runtime
            .publish_topic(
                "devices/d1/status",
                json!({"battery":12,"online":true}),
                Some(&device)
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        runtime
            .execute("Latest", json!({"id":"d1"}), Some(&auth(USER_KEY)))
            .unwrap()["status"],
        Value::Null
    );
}
