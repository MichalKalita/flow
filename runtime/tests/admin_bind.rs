use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread::JoinHandle,
    time::{Duration, Instant},
};

const ADMIN_TOKEN: &str = "admin-bind-integration-key-at-least-32-bytes";
const PROGRAM: &str = r#"[auth [anonymous Anonymous]]
[transport HTTP [auth anonymous]]
[query Health [output Bool] [http GET "/health"] [result true]]"#;

struct Server {
    child: Child,
    directory: PathBuf,
    addresses: Receiver<(String, SocketAddr)>,
    reader: Option<JoinHandle<()>>,
}

impl Server {
    fn start(bind: &str, allow_remote: Option<&str>) -> Self {
        let directory =
            std::env::temp_dir().join(format!("flow-admin-bind-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("application.flow");
        std::fs::write(&source, PROGRAM).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_flow-runtime"));
        command
            .env_clear()
            .env("FLOW_ADMIN_TOKEN", ADMIN_TOKEN)
            .env("FLOW_ADMIN_BIND", bind)
            .env("FLOW_OBSERVABILITY_DIR", directory.join("observability"))
            .env("FLOW_TOKIO_WORKERS", "2")
            .env("FLOW_TOKIO_BLOCKING", "2")
            .arg(source)
            .arg(directory.join("application.sqlite"))
            .args(["127.0.0.1:0", "127.0.0.1:0"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(value) = allow_remote {
            command.env("FLOW_ADMIN_ALLOW_REMOTE", value);
        }
        let mut child = command.spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, addresses) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                for name in ["admin", "HTTP"] {
                    let prefix = format!("Flow {name} listening on http://");
                    if let Some(address) = line.strip_prefix(&prefix) {
                        let mut address: SocketAddr =
                            address.split_whitespace().next().unwrap().parse().unwrap();
                        if address.ip().is_unspecified() {
                            address.set_ip(IpAddr::V4(Ipv4Addr::LOCALHOST));
                        }
                        let _ = sender.send((name.to_owned(), address));
                    }
                }
            }
        });
        Self {
            child,
            directory,
            addresses,
            reader: Some(reader),
        }
    }

    fn listeners(&self) -> (SocketAddr, SocketAddr) {
        let mut admin = None;
        let mut public = None;
        for _ in 0..2 {
            let (name, address) = self
                .addresses
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            match name.as_str() {
                "admin" => admin = Some(address),
                "HTTP" => public = Some(address),
                _ => unreachable!(),
            }
        }
        (admin.unwrap(), public.unwrap())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn get(address: SocketAddr, path: &str, token: Option<&str>) -> u16 {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let authorization = token
        .map(|value| format!("Authorization: Bearer {value}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{authorization}\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response.split_whitespace().nth(1).unwrap().parse().unwrap()
}

#[test]
fn network_admin_requires_explicit_opt_in() {
    for value in [None, Some("0"), Some("true")] {
        let mut server = Server::start("0.0.0.0:0", value);
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = server.child.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "Network admin unexpectedly started"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(!status.success());
        let mut stderr = String::new();
        server
            .child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert!(stderr.contains("FLOW_ADMIN_ALLOW_REMOTE=1"));
        assert!(!server.directory.join("application.sqlite").exists());
    }
}

#[test]
fn network_admin_opt_in_preserves_authentication_and_listener_isolation() {
    let server = Server::start("0.0.0.0:0", Some("1"));
    let (admin, public) = server.listeners();
    assert_ne!(admin.port(), public.port());
    assert_eq!(get(admin, "/api/overview", None), 401);
    assert_eq!(get(admin, "/api/overview", Some("invalid")), 401);
    assert_eq!(get(admin, "/api/overview", Some(ADMIN_TOKEN)), 200);
    assert_eq!(get(public, "/api/overview", Some(ADMIN_TOKEN)), 404);
    assert_eq!(get(public, "/health", None), 200);
}

#[test]
fn loopback_admin_does_not_require_remote_opt_in() {
    let server = Server::start("127.0.0.1:0", None);
    let (admin, _) = server.listeners();
    assert_eq!(get(admin, "/api/overview", None), 401);
    assert_eq!(get(admin, "/api/overview", Some(ADMIN_TOKEN)), 200);
}
