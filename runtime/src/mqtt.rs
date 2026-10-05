use crate::{Error, Result, engine::Runtime};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
fn error(code: &'static str) -> Error {
    Error::new(code, code)
}
pub fn topic_inputs(pattern: &str, topic: &str) -> Result<Option<BTreeMap<String, String>>> {
    if topic.is_empty() || topic.len() > 4096 || topic.contains(['\0', '#', '+']) {
        return Err(error("invalid_input"));
    };
    let a = pattern.split('/').collect::<Vec<_>>();
    let b = topic.split('/').collect::<Vec<_>>();
    if a.len() != b.len() {
        return Ok(None);
    };
    let mut fields = BTreeMap::new();
    for (a, b) in a.iter().zip(b) {
        if let Some(key) = a.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            if b.is_empty() {
                return Ok(None);
            };
            fields.insert(key.into(), b.into());
        } else if *a != b {
            return Ok(None);
        }
    }
    Ok(Some(fields))
}
pub fn matches(filter: &str, topic: &str) -> bool {
    let mut topic = topic.split('/');
    for segment in filter.split('/') {
        if segment == "#" {
            return true;
        };
        let Some(value) = topic.next() else {
            return false;
        };
        if segment != "+" && segment != value {
            return false;
        }
    }
    topic.next().is_none()
}
fn valid_filter(filter: &str) -> bool {
    if filter.is_empty() || filter.len() > 4096 || filter.contains('\0') {
        return false;
    };
    let segments = filter.split('/').collect::<Vec<_>>();
    segments.iter().enumerate().all(|(i, s)| {
        (!s.contains('#') || (*s == "#" && i == segments.len() - 1))
            && (!s.contains('+') || *s == "+")
    })
}
struct Packet {
    header: u8,
    body: Vec<u8>,
}
async fn packet(socket: &mut (impl AsyncRead + Unpin)) -> Result<Packet> {
    let header = socket.read_u8().await.map_err(|_| error("disconnected"))?;
    let mut length = 0usize;
    let mut factor = 1;
    let mut complete = false;
    for _ in 0..4 {
        let byte = socket.read_u8().await.map_err(|_| error("disconnected"))?;
        length += (byte as usize & 127) * factor;
        if byte & 128 == 0 {
            complete = true;
            break;
        };
        factor *= 128;
    }
    if !complete || length > 1024 * 1024 {
        return Err(error("invalid_input"));
    };
    let mut body = vec![0; length];
    socket
        .read_exact(&mut body)
        .await
        .map_err(|_| error("disconnected"))?;
    Ok(Packet { header, body })
}
async fn write(socket: &mut (impl AsyncWrite + Unpin), header: u8, body: &[u8]) -> Result<()> {
    let mut bytes = vec![header];
    let mut length = body.len();
    loop {
        let mut byte = (length % 128) as u8;
        length /= 128;
        if length > 0 {
            byte |= 128
        };
        bytes.push(byte);
        if length == 0 {
            break;
        }
    }
    bytes.extend(body);
    socket
        .write_all(&bytes)
        .await
        .map_err(|_| error("disconnected"))
}
struct Cursor<'a> {
    data: &'a [u8],
    offset: usize,
}
impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }
    fn byte(&mut self) -> Result<u8> {
        let b = *self
            .data
            .get(self.offset)
            .ok_or_else(|| error("invalid_input"))?;
        self.offset += 1;
        Ok(b)
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes([self.byte()?, self.byte()?]))
    }
    fn string(&mut self) -> Result<String> {
        let len = self.u16()? as usize;
        let bytes = self
            .data
            .get(self.offset..self.offset + len)
            .ok_or_else(|| error("invalid_input"))?;
        self.offset += len;
        let value = std::str::from_utf8(bytes).map_err(|_| error("invalid_input"))?;
        if value.contains('\0') {
            return Err(error("invalid_input"));
        };
        Ok(value.into())
    }
}
async fn session(mut socket: TcpStream, runtime: Arc<Mutex<Runtime>>) -> Result<()> {
    let connect = tokio::time::timeout(Duration::from_secs(10), packet(&mut socket))
        .await
        .map_err(|_| error("disconnected"))??;
    if connect.header != 0x10 {
        return Err(error("invalid_input"));
    };
    let mut cursor = Cursor::new(&connect.body);
    if cursor.string()? != "MQTT" || cursor.byte()? != 4 {
        return Err(error("invalid_input"));
    };
    let flags = cursor.byte()?;
    let keepalive = cursor.u16()?;
    let client_id = cursor.string()?;
    if client_id.len() > 256 || flags & 1 != 0 || flags & 4 != 0 || flags & 2 == 0 {
        return Err(error("invalid_input"));
    };
    let alias = if flags & 128 != 0 {
        Some(cursor.string()?)
    } else {
        None
    };
    let secret = if flags & 64 != 0 {
        Some(cursor.string()?)
    } else {
        None
    };
    if cursor.offset != connect.body.len() {
        return Err(error("invalid_input"));
    };
    let credential = {
        let guard = runtime.lock().map_err(|_| error("internal"))?;
        match (alias, secret) {
            (Some(a), Some(s)) => Some(guard.mqtt_credential(&a, &s)?),
            (None, None) => None,
            _ => return Err(error("unauthenticated")),
        }
    };
    if runtime
        .lock()
        .map_err(|_| error("internal"))?
        .authenticate_transport(credential.as_deref(), "MQTT")
        .is_err()
    {
        write(&mut socket, 0x20, &[0, 5]).await?;
        return Err(error("unauthenticated"));
    };
    write(&mut socket, 0x20, &[0, 0]).await?;
    let (mut reader, mut socket) = socket.into_split();
    let (sender, mut incoming) = tokio::sync::mpsc::channel(32);
    let reader_task = tokio::spawn(async move {
        loop {
            let result = packet(&mut reader).await;
            let failed = result.is_err();
            if sender.send(result).await.is_err() || failed {
                return;
            }
        }
    });
    struct StopReader(tokio::task::JoinHandle<()>);
    impl Drop for StopReader {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _reader = StopReader(reader_task);
    let mut filters = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut last = tokio::time::Instant::now();
    loop {
        let event = tokio::select! {
            value = incoming.recv() => Some(value),
            _ = tick.tick() => None,
        };
        match event {
            Some(p) => {
                let p = p.ok_or_else(|| error("disconnected"))??;
                last = tokio::time::Instant::now();
                match p.header >> 4 {
                    3 => {
                        let qos = (p.header >> 1) & 3;
                        if qos > 1 || p.header & 1 != 0 {
                            return Err(error("invalid_input"));
                        };
                        let mut cursor = Cursor::new(&p.body);
                        let topic = cursor.string()?;
                        let id = if qos == 1 { Some(cursor.u16()?) } else { None };
                        let payload: serde_json::Value =
                            serde_json::from_slice(&p.body[cursor.offset..])?;
                        let runtime = runtime.clone();
                        let credential = credential.clone();
                        tokio::task::spawn_blocking(move || {
                            runtime
                                .lock()
                                .map_err(|_| error("internal"))?
                                .publish_topic(&topic, payload, credential.as_deref())
                        })
                        .await
                        .map_err(|_| error("internal"))??;
                        if let Some(id) = id {
                            if id == 0 {
                                return Err(error("invalid_input"));
                            };
                            write(&mut socket, 0x40, &id.to_be_bytes()).await?;
                        }
                    }
                    8 => {
                        if p.header != 0x82 {
                            return Err(error("invalid_input"));
                        };
                        let mut cursor = Cursor::new(&p.body);
                        let id = cursor.u16()?;
                        if id == 0 {
                            return Err(error("invalid_input"));
                        };
                        let mut codes = vec![];
                        while cursor.offset < p.body.len() {
                            let filter = cursor.string()?;
                            let qos = cursor.byte()?;
                            if qos > 1 || !valid_filter(&filter) || filters.len() >= 32 {
                                codes.push(128)
                            } else {
                                filters.insert(filter);
                                codes.push(0)
                            }
                        }
                        if codes.is_empty() {
                            return Err(error("invalid_input"));
                        };
                        let mut body = id.to_be_bytes().to_vec();
                        body.extend(codes);
                        write(&mut socket, 0x90, &body).await?;
                    }
                    10 => {
                        if p.header != 0xa2 {
                            return Err(error("invalid_input"));
                        };
                        let mut cursor = Cursor::new(&p.body);
                        let id = cursor.u16()?;
                        while cursor.offset < p.body.len() {
                            filters.remove(&cursor.string()?);
                        }
                        write(&mut socket, 0xb0, &id.to_be_bytes()).await?;
                    }
                    12 => {
                        if p.header != 0xc0 || !p.body.is_empty() {
                            return Err(error("invalid_input"));
                        };
                        runtime
                            .lock()
                            .map_err(|_| error("internal"))?
                            .authenticate_transport(credential.as_deref(), "MQTT")?;
                        write(&mut socket, 0xd0, &[]).await?;
                    }
                    14 => return Ok(()),
                    _ => return Err(error("invalid_input")),
                }
            }
            None => {
                if keepalive > 0 && last.elapsed() > Duration::from_secs(keepalive as u64 * 3 / 2) {
                    return Err(error("disconnected"));
                };
                let runtime = runtime.clone();
                let credential = credential.clone();
                let filters = filters.clone();
                let messages = tokio::task::spawn_blocking(move || {
                    let mut guard = runtime.lock().map_err(|_| error("internal"))?;
                    guard.authenticate_transport(credential.as_deref(), "MQTT")?;
                    let mut messages = vec![];
                    for filter in filters {
                        messages.extend(guard.mqtt_messages(&filter, credential.as_deref())?)
                    }
                    Ok::<_, Error>(messages)
                })
                .await
                .map_err(|_| error("internal"))??;
                for (id, topic, payload) in messages {
                    if seen.insert(id) {
                        let mut body = (topic.len() as u16).to_be_bytes().to_vec();
                        body.extend(topic.as_bytes());
                        body.extend(payload.to_string().as_bytes());
                        write(&mut socket, 0x30, &body).await?;
                    }
                }
                if seen.len() > 100000 {
                    return Err(error("limit"));
                };
            }
        }
    }
}
pub async fn serve(listener: TcpListener, runtime: Arc<Mutex<Runtime>>) -> std::io::Result<()> {
    loop {
        let (socket, _) = listener.accept().await?;
        let runtime = runtime.clone();
        tokio::spawn(async move {
            if let Err(e) = session(socket, runtime).await
                && e.code != "disconnected"
            {
                eprintln!("MQTT: {}", e.code)
            }
        });
    }
}
