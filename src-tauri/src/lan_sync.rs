use std::{
    collections::{hash_map::RandomState, HashMap},
    hash::{BuildHasher, Hasher},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::{watch, Notify},
    time::Instant,
};

use crate::backup::{self, BackupPayload, ImportResult};

pub const SYNC_DEFAULT_PORT: u16 = 17580;
pub const SYNC_DISCOVERY_PORT: u16 = 17581;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_SYNC_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_FAILED_PIN_ATTEMPTS: u8 = 5;
const SERVER_IDLE_TIMEOUT_SECS: u64 = 600;
const REQUEST_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LanServerInfo {
    pub ip: String,
    pub port: u16,
    pub address: String,
    pub pin: String,
    pub device_name: String,
    pub platform: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LanPeer {
    pub device_name: String,
    pub platform: String,
    pub ip: String,
    pub port: u16,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LanSyncOutcome {
    pub remote_device_name: String,
    pub local_import: ImportResult,
    pub remote_import: ImportResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LanSyncReceivedEvent {
    pub remote_device_name: String,
    pub local_import: ImportResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncHttpResponse {
    remote_device_name: String,
    remote_import: ImportResult,
    merged_backup: BackupPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveryBeacon {
    app: String,
    session_id: String,
    device_name: String,
    platform: String,
    ip: String,
    port: u16,
}

#[derive(Debug, Deserialize)]
struct DiscoveryProbe {
    app: String,
    probe: u8,
}

pub struct SyncServerHandle {
    info: LanServerInfo,
    session_id: String,
    shutdown_tx: watch::Sender<bool>,
}

impl SyncServerHandle {
    pub fn info(&self) -> &LanServerInfo {
        &self.info
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn is_stopped(&self) -> bool {
        *self.shutdown_tx.borrow()
    }

    pub fn stop(&self) {
        let _ = self.shutdown_tx.send(true);
    }
}

impl Drop for SyncServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn random_u64() -> u64 {
    RandomState::new().build_hasher().finish()
}

pub fn generate_pin() -> String {
    format!("{:06}", random_u64() % 1_000_000)
}

fn generate_session_id() -> String {
    format!("{:016x}{:016x}", random_u64(), random_u64())
}

pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    if ab.len() != bb.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in ab.iter().zip(bb.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub fn local_platform() -> &'static str {
    if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        std::env::consts::OS
    }
}

pub fn local_device_name() -> String {
    if cfg!(target_os = "android") {
        return "Android 移动端".to_string();
    }
    for key in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(name) = std::env::var(key) {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                return if cfg!(target_os = "windows") {
                    format!("{trimmed} (Windows)")
                } else {
                    trimmed.to_string()
                };
            }
        }
    }
    if cfg!(target_os = "windows") {
        "Windows 桌面端".to_string()
    } else {
        "Trace 设备".to_string()
    }
}

fn is_usable_lan_ipv4(ip: Ipv4Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
        return false;
    }
    let o = ip.octets();
    !(o[0] == 169 && o[1] == 254)
}

pub fn detect_lan_ipv4() -> Option<Ipv4Addr> {
    let candidates = [
        "192.168.1.1:80",
        "192.168.0.1:80",
        "10.0.0.1:80",
        "172.16.0.1:80",
        "223.5.5.5:80",
        "8.8.8.8:80",
    ];
    let mut fallback: Option<Ipv4Addr> = None;
    for target in candidates {
        let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") else {
            continue;
        };
        if sock.connect(target).is_err() {
            continue;
        }
        if let Ok(SocketAddr::V4(local)) = sock.local_addr() {
            let ip = *local.ip();
            if !is_usable_lan_ipv4(ip) {
                continue;
            }
            if ip.is_private() {
                return Some(ip);
            }
            if fallback.is_none() {
                fallback = Some(ip);
            }
        }
    }
    fallback
}

fn subnet_broadcast_targets(local_ip: Option<Ipv4Addr>) -> Vec<SocketAddr> {
    let mut targets = vec![SocketAddr::from((Ipv4Addr::BROADCAST, SYNC_DISCOVERY_PORT))];
    if let Some(ip) = local_ip {
        let o = ip.octets();
        let directed = Ipv4Addr::new(o[0], o[1], o[2], 255);
        if directed != Ipv4Addr::BROADCAST {
            targets.push(SocketAddr::from((directed, SYNC_DISCOVERY_PORT)));
        }
    }
    targets
}

pub fn normalize_peer_address(raw: &str) -> Result<String, String> {
    let mut trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("请填写对方设备的连接地址".to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("http://") {
        trimmed = rest;
    } else if let Some(rest) = trimmed.strip_prefix("https://") {
        trimmed = rest;
    }
    if let Some((host_port, _)) = trimmed.split_once('/') {
        trimmed = host_port;
    }
    let trimmed = trimmed.trim();
    if trimmed.is_empty() {
        return Err("连接地址格式无效".to_string());
    }

    if let Some((host, port_str)) = trimmed.rsplit_once(':') {
        if host.is_empty() || host.contains(char::is_whitespace) {
            return Err("连接地址中的主机名或 IP 无效".to_string());
        }
        let port: u16 = port_str
            .parse()
            .map_err(|_| format!("端口号无效：{port_str}"))?;
        if port == 0 {
            return Err("端口号不能为 0".to_string());
        }
        Ok(format!("{host}:{port}"))
    } else {
        if trimmed.contains(char::is_whitespace) {
            return Err("连接地址中不能包含空格".to_string());
        }
        Ok(format!("{trimmed}:{SYNC_DEFAULT_PORT}"))
    }
}

pub async fn start_server<F>(
    db: Arc<Mutex<Connection>>,
    on_received: F,
) -> Result<(SyncServerHandle, LanServerInfo), String>
where
    F: Fn(LanSyncReceivedEvent) + Send + Sync + 'static,
{
    let listener = match TcpListener::bind(("0.0.0.0", SYNC_DEFAULT_PORT)).await {
        Ok(l) => l,
        Err(_) => TcpListener::bind(("0.0.0.0", 0))
            .await
            .map_err(|e| format!("启动局域网快传服务失败：{e}"))?,
    };
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取快传端口失败：{e}"))?
        .port();

    let lan_ip = detect_lan_ipv4();
    let ip_str = lan_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let address = format!("{ip_str}:{port}");
    let pin = generate_pin();
    let session_id = generate_session_id();
    let device_name = local_device_name();
    let platform = local_platform().to_string();

    let info = LanServerInfo {
        ip: ip_str.clone(),
        port,
        address,
        pin: pin.clone(),
        device_name: device_name.clone(),
        platform: platform.clone(),
    };

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let activity_notify = Arc::new(Notify::new());
    let failed_attempts = Arc::new(AtomicU8::new(0));
    let callback = Arc::new(on_received);

    // 启动局域网设备发现信标与探测响应任务（绝不包含 PIN）
    let beacon = DiscoveryBeacon {
        app: "trace".to_string(),
        session_id: session_id.clone(),
        device_name: device_name.clone(),
        platform: platform.clone(),
        ip: ip_str.clone(),
        port,
    };
    tokio::spawn(run_discovery_responder(
        beacon,
        lan_ip,
        shutdown_rx.clone(),
    ));

    // 启动 HTTP/1.1 快传服务主循环
    let server_info = info.clone();
    let server_shutdown_tx = shutdown_tx.clone();
    tokio::spawn(async move {
        let mut rx = shutdown_rx;
        let idle_duration = Duration::from_secs(SERVER_IDLE_TIMEOUT_SECS);
        let mut deadline = Instant::now() + idle_duration;

        loop {
            tokio::select! {
                _ = rx.changed() => {
                    if *rx.borrow() {
                        break;
                    }
                }
                _ = activity_notify.notified() => {
                    deadline = Instant::now() + idle_duration;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    let _ = server_shutdown_tx.send(true);
                    break;
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _peer_addr)) => {
                            let db_ref = Arc::clone(&db);
                            let info_ref = server_info.clone();
                            let attempts_ref = Arc::clone(&failed_attempts);
                            let notify_ref = Arc::clone(&activity_notify);
                            let shutdown_ref = server_shutdown_tx.clone();
                            let cb_ref = Arc::clone(&callback);

                            tokio::spawn(async move {
                                let _ = tokio::time::timeout(
                                    Duration::from_secs(REQUEST_TIMEOUT_SECS),
                                    handle_http_connection(
                                        stream,
                                        db_ref,
                                        info_ref,
                                        attempts_ref,
                                        notify_ref,
                                        shutdown_ref,
                                        cb_ref,
                                    ),
                                )
                                .await;
                            });
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    });

    Ok((
        SyncServerHandle {
            info: info.clone(),
            session_id,
            shutdown_tx,
        },
        info,
    ))
}

async fn run_discovery_responder(
    beacon: DiscoveryBeacon,
    lan_ip: Option<Ipv4Addr>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let socket = match UdpSocket::bind(("0.0.0.0", SYNC_DISCOVERY_PORT)).await {
        Ok(s) => s,
        Err(_) => match UdpSocket::bind("0.0.0.0:0").await {
            Ok(s) => s,
            Err(_) => return,
        },
    };
    let _ = socket.set_broadcast(true);
    let Ok(payload) = serde_json::to_vec(&beacon) else {
        return;
    };
    let targets = subnet_broadcast_targets(lan_ip);
    let mut recv_buf = [0u8; 512];
    let mut tick = tokio::time::interval(Duration::from_secs(1));

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            _ = tick.tick() => {
                for target in &targets {
                    let _ = socket.send_to(&payload, target).await;
                }
            }
            recv_res = socket.recv_from(&mut recv_buf) => {
                if let Ok((len, src)) = recv_res {
                    if let Ok(probe) = serde_json::from_slice::<DiscoveryProbe>(&recv_buf[..len]) {
                        if probe.app == "trace" && probe.probe == 1 {
                            let _ = socket.send_to(&payload, src).await;
                        }
                    }
                }
            }
        }
    }
}

async fn handle_http_connection<F>(
    mut stream: TcpStream,
    db: Arc<Mutex<Connection>>,
    info: LanServerInfo,
    failed_attempts: Arc<AtomicU8>,
    activity_notify: Arc<Notify>,
    shutdown_tx: watch::Sender<bool>,
    on_received: Arc<F>,
) where
    F: Fn(LanSyncReceivedEvent) + Send + Sync + 'static,
{
    let mut buf = Vec::<u8>::with_capacity(4096);
    let mut chunk = [0u8; 4096];

    let header_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if buf.len() >= MAX_HEADER_BYTES {
            let _ = write_http_json(
                &mut stream,
                400,
                "Bad Request",
                r#"{"error":"请求头过大"}"#,
            )
            .await;
            return;
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    };

    let header_bytes = &buf[..header_end];
    let header_text = String::from_utf8_lossy(header_bytes);
    let mut lines = header_text.split("\r\n");
    let Some(request_line) = lines.next() else {
        let _ = write_http_json(&mut stream, 400, "Bad Request", r#"{"error":"无效请求"}"#).await;
        return;
    };

    let mut req_parts = request_line.split_whitespace();
    let method = req_parts.next().unwrap_or("");
    let path = req_parts.next().unwrap_or("");

    let mut headers: HashMap<String, String> = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }

    if method == "GET" && path == "/api/ping" {
        let body = serde_json::json!({
            "app": "trace",
            "deviceName": info.device_name,
            "platform": info.platform,
            "port": info.port,
        })
        .to_string();
        let _ = write_http_json(&mut stream, 200, "OK", &body).await;
        return;
    }

    if !(method == "POST" && path == "/api/sync") {
        let _ = write_http_json(
            &mut stream,
            404,
            "Not Found",
            r#"{"error":"未知接口"}"#,
        )
        .await;
        return;
    }

    let content_length: usize = match headers.get("content-length") {
        Some(raw_len) => match raw_len.parse::<usize>() {
            Ok(n) => n,
            Err(_) => {
                let _ = write_http_json(
                    &mut stream,
                    400,
                    "Bad Request",
                    r#"{"error":"Content-Length 无效"}"#,
                )
                .await;
                return;
            }
        },
        None => 0,
    };

    if content_length > MAX_SYNC_BODY_BYTES {
        let _ = write_http_json(
            &mut stream,
            413,
            "Payload Too Large",
            r#"{"error":"同步数据超过 32 MB 限制"}"#,
        )
        .await;
        return;
    }

    let mut body_buf = buf.split_off(header_end + 4);
    while body_buf.len() < content_length {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => {
                let _ = write_http_json(
                    &mut stream,
                    400,
                    "Bad Request",
                    r#"{"error":"请求体读取中断"}"#,
                )
                .await;
                return;
            }
            Ok(n) => body_buf.extend_from_slice(&chunk[..n]),
        }
    }
    body_buf.truncate(content_length);

    let provided_pin = headers
        .get("x-trace-pin")
        .map(|s| s.trim())
        .unwrap_or("");
    if !constant_time_eq(provided_pin, &info.pin) {
        let tries = failed_attempts.fetch_add(1, Ordering::SeqCst) + 1;
        if tries >= MAX_FAILED_PIN_ATTEMPTS {
            let _ = write_http_json(
                &mut stream,
                429,
                "Too Many Requests",
                r#"{"error":"配对码连续错误 5 次，本机已自动关闭本次快传服务，请重新开启"}"#,
            )
            .await;
            let _ = shutdown_tx.send(true);
        } else {
            let remaining = MAX_FAILED_PIN_ATTEMPTS.saturating_sub(tries);
            let err_json = serde_json::json!({
                "error": format!("配对码错误，还可尝试 {remaining} 次")
            })
            .to_string();
            let _ = write_http_json(&mut stream, 401, "Unauthorized", &err_json).await;
        }
        return;
    }

    failed_attempts.store(0, Ordering::SeqCst);
    activity_notify.notify_one();

    let remote_device_name = headers
        .get("x-trace-device")
        .map(|s| decode_header_value(s))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "局域网设备".to_string());

    let body_text = match String::from_utf8(body_buf) {
        Ok(s) => s,
        Err(_) => {
            let _ = write_http_json(
                &mut stream,
                400,
                "Bad Request",
                r#"{"error":"请求内容必须是 UTF-8 JSON"}"#,
            )
            .await;
            return;
        }
    };

    let merge_res: Result<(ImportResult, BackupPayload), String> =
        tokio::task::spawn_blocking(move || {
            let mut conn = db.lock().map_err(|_| "数据库锁定失败".to_string())?;
            let remote_import = backup::import_backup_json(&mut conn, &body_text, false)?;
            let merged_json = backup::export_backup_json(&conn)?;
            let merged_backup: BackupPayload = serde_json::from_str(&merged_json)
                .map_err(|e| format!("序列化合并备份失败：{e}"))?;
            Ok((remote_import, merged_backup))
        })
        .await
        .map_err(|e| format!("执行数据库合并任务失败：{e}"))
        .and_then(|r| r);

    match merge_res {
        Ok((remote_import, merged_backup)) => {
            on_received(LanSyncReceivedEvent {
                remote_device_name: remote_device_name.clone(),
                local_import: remote_import.clone(),
            });
            let resp = SyncHttpResponse {
                remote_device_name: info.device_name.clone(),
                remote_import,
                merged_backup,
            };
            let resp_json = match serde_json::to_string(&resp) {
                Ok(j) => j,
                Err(e) => {
                    let err_json = serde_json::json!({
                        "error": format!("序列化响应失败：{e}")
                    })
                    .to_string();
                    let _ = write_http_json(&mut stream, 500, "Internal Server Error", &err_json)
                        .await;
                    return;
                }
            };
            let _ = write_http_json(&mut stream, 200, "OK", &resp_json).await;
        }
        Err(err) => {
            let err_json = serde_json::json!({ "error": err }).to_string();
            let _ = write_http_json(&mut stream, 400, "Bad Request", &err_json).await;
        }
    }
}

fn encode_header_value(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.as_bytes() {
        let ch = *b as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~') {
            out.push(ch);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn decode_header_value(raw: &str) -> String {
    let bytes = raw.trim().as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).trim().to_string()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

async fn write_http_json(
    stream: &mut TcpStream,
    status_code: u16,
    reason: &str,
    json_body: &str,
) -> std::io::Result<()> {
    let bytes = json_body.as_bytes();
    let head = format!(
        "HTTP/1.1 {status_code} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(bytes).await?;
    stream.flush().await
}

pub async fn discover_peers(
    timeout_ms: u64,
    own_session_id: Option<&str>,
) -> Result<Vec<LanPeer>, String> {
    let wait_ms = timeout_ms.clamp(500, 5000);
    let socket = match UdpSocket::bind(("0.0.0.0", SYNC_DISCOVERY_PORT)).await {
        Ok(s) => s,
        Err(_) => UdpSocket::bind("0.0.0.0:0")
            .await
            .map_err(|e| format!("初始化局域网扫描失败：{e}"))?,
    };
    let _ = socket.set_broadcast(true);

    let probe = br#"{"app":"trace","probe":1}"#;
    let lan_ip = detect_lan_ipv4();
    let targets = subnet_broadcast_targets(lan_ip);
    for target in &targets {
        let _ = socket.send_to(probe, target).await;
    }

    let deadline = Instant::now() + Duration::from_millis(wait_ms);
    let mut peers: HashMap<String, LanPeer> = HashMap::new();
    let mut buf = [0u8; 2048];
    let mut second_probe_sent = false;

    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        if !second_probe_sent && deadline - now <= Duration::from_millis(wait_ms / 2) {
            second_probe_sent = true;
            for target in &targets {
                let _ = socket.send_to(probe, target).await;
            }
        }

        let remaining = deadline - now;
        match tokio::time::timeout(remaining, socket.recv_from(&mut buf)).await {
            Ok(Ok((len, src_addr))) => {
                let Ok(beacon) = serde_json::from_slice::<DiscoveryBeacon>(&buf[..len]) else {
                    continue;
                };
                if beacon.app != "trace" || beacon.port == 0 {
                    continue;
                }
                if own_session_id.is_some_and(|sid| sid == beacon.session_id) {
                    continue;
                }
                let peer_ip = match src_addr.ip() {
                    IpAddr::V4(v4) if is_usable_lan_ipv4(v4) => v4.to_string(),
                    _ => beacon.ip,
                };
                let address = format!("{peer_ip}:{}", beacon.port);
                peers.insert(
                    address.clone(),
                    LanPeer {
                        device_name: beacon.device_name,
                        platform: beacon.platform,
                        ip: peer_ip,
                        port: beacon.port,
                        address,
                    },
                );
            }
            Ok(Err(_)) | Err(_) => break,
        }
    }

    let mut list: Vec<LanPeer> = peers.into_values().collect();
    list.sort_by(|a, b| a.device_name.cmp(&b.device_name).then(a.address.cmp(&b.address)));
    Ok(list)
}

pub async fn sync_with_peer(
    db: Arc<Mutex<Connection>>,
    address: &str,
    pin: &str,
) -> Result<LanSyncOutcome, String> {
    let normalized_addr = normalize_peer_address(address)?;
    let clean_pin = pin.trim();
    if clean_pin.len() != 6 || !clean_pin.bytes().all(|b| b.is_ascii_digit()) {
        return Err("请输入 6 位数字配对码".to_string());
    }

    let db_for_export = Arc::clone(&db);
    let local_backup_json = tokio::task::spawn_blocking(move || {
        let conn = db_for_export
            .lock()
            .map_err(|_| "数据库锁定失败".to_string())?;
        backup::export_backup_json(&conn)
    })
    .await
    .map_err(|e| format!("导出本地数据失败：{e}"))??;

    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("初始化局域网客户端失败：{e}"))?;

    let url = format!("http://{normalized_addr}/api/sync");
    let device_header = encode_header_value(&local_device_name());

    let resp = client
        .post(&url)
        .header("Content-Type", "application/json; charset=utf-8")
        .header("X-Trace-Pin", clean_pin)
        .header("X-Trace-Device", device_header)
        .body(local_backup_json)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "连接对方设备超时：请确认双方处于同一局域网 Wi-Fi，且对方未锁屏或被系统防火墙拦截".to_string()
            } else if e.is_connect() {
                "无法连接到对方设备：请确认对方已开启局域网快传、地址端口填写正确，且双方连接同一个 Wi-Fi".to_string()
            } else {
                format!("局域网同步请求失败：{e}")
            }
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().await.unwrap_or_default();
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&err_body) {
            if let Some(msg) = val.get("error").and_then(|v| v.as_str()) {
                return Err(msg.to_string());
            }
        }
        return Err(format!("对方设备返回异常状态（{status}）"));
    }

    let sync_resp: SyncHttpResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析对方同步响应失败：{e}"))?;

    let merged_json = serde_json::to_string(&sync_resp.merged_backup)
        .map_err(|e| format!("序列化同步结果失败：{e}"))?;

    let local_import = tokio::task::spawn_blocking(move || {
        let mut conn = db.lock().map_err(|_| "数据库锁定失败".to_string())?;
        backup::import_backup_json(&mut conn, &merged_json, false)
    })
    .await
    .map_err(|e| format!("写入本地合并数据失败：{e}"))??;

    Ok(LanSyncOutcome {
        remote_device_name: sync_resp.remote_device_name,
        local_import,
        remote_import: sync_resp.remote_import,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, AppSettings};

    fn mem_db_arc() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        Arc::new(Mutex::new(conn))
    }

    #[test]
    fn normalizes_peer_addresses_and_rejects_invalid() {
        assert_eq!(
            normalize_peer_address("192.168.1.25").unwrap(),
            "192.168.1.25:17580"
        );
        assert_eq!(
            normalize_peer_address("http://192.168.1.25:18000/").unwrap(),
            "192.168.1.25:18000"
        );
        assert!(normalize_peer_address("   ").is_err());
        assert!(normalize_peer_address("192.168.1.25:0").is_err());
        assert!(normalize_peer_address("192.168.1.25:abc").is_err());
    }

    #[tokio::test]
    async fn end_to_end_bidirectional_sync_merges_without_leaking_settings() {
        let db_server = mem_db_arc();
        let db_client = mem_db_arc();

        {
            let conn = db_server.lock().unwrap();
            db::save_settings(
                &conn,
                &AppSettings {
                    gemini_api_key: "AIzaServerSecretKey1234567890".to_string(),
                    gemini_model: "gemini-3.8-flash".to_string(),
                    gemini_base_url: String::new(),
                    theme: "dark".to_string(),
                },
            )
            .unwrap();
            db::upsert_entry(&conn, "2026-11-19", "服务端旧版").unwrap();
            db::upsert_entry(&conn, "2026-11-20", "服务端独有日记").unwrap();
            db::save_ai_report(
                &conn,
                "week",
                "2026-W47",
                "2026-11-16",
                "2026-11-22",
                "gemini-3.8-flash",
                "总结",
                "服务端周报",
            )
            .unwrap();
        }

        {
            let conn = db_client.lock().unwrap();
            db::save_settings(
                &conn,
                &AppSettings {
                    gemini_api_key: "AIzaClientSecretKey1234567890".to_string(),
                    gemini_model: "gemini-3.6-flash".to_string(),
                    gemini_base_url: String::new(),
                    theme: "light".to_string(),
                },
            )
            .unwrap();
            db::upsert_entry(&conn, "2026-11-18", "客户端独有日记").unwrap();
            // 让客户端 2026-11-19 的 updated_at 严格大于服务端
            conn.execute(
                "INSERT INTO entries (entry_date, content, word_count, created_at, updated_at)
                 VALUES ('2026-11-19', '客户端更新版', 6, 100, 9999999999)",
                [],
            )
            .unwrap();
            db::save_ai_report(
                &conn,
                "day",
                "2026-11-18",
                "2026-11-18",
                "2026-11-18",
                "gemini-3.6-flash",
                "今日",
                "客户端日报",
            )
            .unwrap();
        }

        let received_events = Arc::new(Mutex::new(Vec::<LanSyncReceivedEvent>::new()));
        let events_clone = Arc::clone(&received_events);
        let (handle, info) = start_server(Arc::clone(&db_server), move |evt| {
            events_clone.lock().unwrap().push(evt);
        })
        .await
        .unwrap();

        let local_addr = format!("127.0.0.1:{}", info.port);
        let outcome = sync_with_peer(Arc::clone(&db_client), &local_addr, &info.pin)
            .await
            .unwrap();

        assert_eq!(outcome.remote_import.imported_entries, 1);
        assert_eq!(outcome.remote_import.updated_entries, 1);
        assert_eq!(outcome.local_import.imported_entries, 1);
        assert_eq!(outcome.local_import.imported_reports, 1);

        {
            let conn = db_server.lock().unwrap();
            assert_eq!(
                db::get_entry(&conn, "2026-11-18").unwrap().unwrap().content,
                "客户端独有日记"
            );
            assert_eq!(
                db::get_entry(&conn, "2026-11-19").unwrap().unwrap().content,
                "客户端更新版"
            );
            assert_eq!(
                db::get_entry(&conn, "2026-11-20").unwrap().unwrap().content,
                "服务端独有日记"
            );
            assert_eq!(db::list_ai_reports(&conn, 10).unwrap().len(), 2);
            assert_eq!(
                db::get_settings(&conn).unwrap().gemini_api_key,
                "AIzaServerSecretKey1234567890"
            );
        }

        {
            let conn = db_client.lock().unwrap();
            assert_eq!(
                db::get_entry(&conn, "2026-11-20").unwrap().unwrap().content,
                "服务端独有日记"
            );
            assert_eq!(
                db::get_entry(&conn, "2026-11-19").unwrap().unwrap().content,
                "客户端更新版"
            );
            assert_eq!(db::list_ai_reports(&conn, 10).unwrap().len(), 2);
            assert_eq!(
                db::get_settings(&conn).unwrap().gemini_api_key,
                "AIzaClientSecretKey1234567890"
            );
        }

        assert_eq!(received_events.lock().unwrap().len(), 1);
        handle.stop();
    }

    #[tokio::test]
    async fn rejects_wrong_pin_and_locks_after_five_failures() {
        let db_server = mem_db_arc();
        let db_client = mem_db_arc();

        {
            let conn = db_client.lock().unwrap();
            db::upsert_entry(&conn, "2026-11-21", "不应被写入服务端").unwrap();
        }

        let (handle, info) = start_server(Arc::clone(&db_server), |_| {})
            .await
            .unwrap();
        let local_addr = format!("127.0.0.1:{}", info.port);
        let wrong_pin = if info.pin == "000000" {
            "999999"
        } else {
            "000000"
        };

        for attempt in 1..=5 {
            let err = sync_with_peer(Arc::clone(&db_client), &local_addr, wrong_pin)
                .await
                .unwrap_err();
            if attempt < 5 {
                assert!(err.contains("配对码错误"), "unexpected err: {err}");
            } else {
                assert!(err.contains("自动关闭"), "unexpected lock err: {err}");
            }
        }

        // 验证错 PIN 未污染服务端数据库
        {
            let conn = db_server.lock().unwrap();
            assert!(db::get_entry(&conn, "2026-11-21").unwrap().is_none());
        }

        // 服务自动关闭后，即使使用正确 PIN 也无法继续连接
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(sync_with_peer(Arc::clone(&db_client), &local_addr, &info.pin)
            .await
            .is_err());

        handle.stop();
    }
}
