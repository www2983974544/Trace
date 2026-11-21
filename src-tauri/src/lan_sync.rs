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
    sync::{mpsc, watch, Notify},
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
    pub addresses: Vec<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanInterface {
    pub name: String,
    pub description: String,
    pub ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub broadcast: Ipv4Addr,
    pub priority: i32,
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

pub fn is_rfc1918_private(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 10 || (o[0] == 172 && (16..=31).contains(&o[1])) || (o[0] == 192 && o[1] == 168)
}

pub fn is_clash_fake_ip(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 198 && (o[1] == 18 || o[1] == 19)
}

pub fn is_carrier_cgnat(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (64..=127).contains(&o[1])
}

pub fn netmask_from_prefix(prefix_len: u8) -> Ipv4Addr {
    if prefix_len == 0 {
        return Ipv4Addr::UNSPECIFIED;
    }
    let mask = if prefix_len >= 32 {
        u32::MAX
    } else {
        u32::MAX << (32 - prefix_len)
    };
    Ipv4Addr::from(mask)
}

#[cfg(any(unix, test))]
pub fn prefix_from_netmask(mask: Ipv4Addr) -> Option<u8> {
    let raw = u32::from(mask);
    let ones = raw.count_ones() as u8;
    if raw == u32::from(netmask_from_prefix(ones)) {
        Some(ones)
    } else {
        None
    }
}

pub fn compute_broadcast(ip: Ipv4Addr, netmask: Ipv4Addr) -> Ipv4Addr {
    let ip_u32 = u32::from(ip);
    let mask_u32 = u32::from(netmask);
    Ipv4Addr::from(ip_u32 | !mask_u32)
}

pub fn same_subnet(a: Ipv4Addr, b: Ipv4Addr, netmask: Ipv4Addr) -> bool {
    let mask = u32::from(netmask);
    (u32::from(a) & mask) == (u32::from(b) & mask)
}

pub fn is_virtual_or_cellular_adapter(name: &str, desc: &str) -> bool {
    let name_low = name.trim().to_ascii_lowercase();
    let desc_low = desc.trim().to_ascii_lowercase();
    let combined = format!("{name_low} {desc_low}");

    let exact_prefixes = ["tun", "utun", "wg", "tap", "ppp"];
    if exact_prefixes.iter().any(|p| name_low.starts_with(p)) {
        return true;
    }

    let blocked_tokens = [
        // 代理 / VPN / 隧道
        "mihomo",
        "meta tunnel",
        "clash",
        "sing-box",
        "wintun",
        "wireguard",
        "tailscale",
        "zerotier",
        "openvpn",
        "v2ray",
        "nekobox",
        "flclash",
        "tunnel",
        "tap-windows",
        // 虚拟机 / 容器 / 蓝牙 / 回环
        "vmware",
        "vmnet",
        "virtualbox",
        "vbox",
        "hyper-v",
        "vethernet",
        "wsl",
        "docker",
        "podman",
        "bluetooth",
        "蓝牙",
        "loopback",
        "isatap",
        "teredo",
        "npcap",
        // 蜂窝移动流量与系统虚拟接口
        "rmnet",
        "ccmni",
        "pdp",
        "wwan",
        "cdma",
        "cellular",
        "mobile",
        "ims",
        "clat",
        "seth_lte",
        "epdg",
        "dummy",
    ];

    blocked_tokens.iter().any(|tok| combined.contains(tok))
}

fn is_allowed_lan_peer_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_rfc1918_private(v4) || v4.is_loopback(),
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return true;
            }
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_rfc1918_private(mapped) || mapped.is_loopback();
            }
            false
        }
    }
}

fn sort_and_dedup_interfaces(mut list: Vec<LanInterface>) -> Vec<LanInterface> {
    list.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.ip.cmp(&b.ip))
    });
    let mut seen_ips = Vec::with_capacity(list.len());
    list.retain(|iface| {
        if seen_ips.contains(&iface.ip) {
            false
        } else {
            seen_ips.push(iface.ip);
            true
        }
    });
    list
}

#[cfg(windows)]
unsafe fn wide_ptr_to_string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    let slice = std::slice::from_raw_parts(ptr, len);
    String::from_utf16_lossy(slice)
}

#[cfg(windows)]
pub fn enumerate_lan_interfaces() -> Vec<LanInterface> {
    use windows_sys::Win32::{
        Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS},
        NetworkManagement::{
            IpHelper::{
                GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST,
                GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IF_TYPE_ETHERNET_CSMACD,
                IF_TYPE_IEEE80211, IP_ADAPTER_ADDRESSES_LH,
            },
            Ndis::IfOperStatusUp,
        },
        Networking::WinSock::{AF_INET, SOCKADDR_IN},
    };

    let flags = GAA_FLAG_SKIP_ANYCAST
        | GAA_FLAG_SKIP_MULTICAST
        | GAA_FLAG_SKIP_DNS_SERVER
        | GAA_FLAG_INCLUDE_GATEWAYS;
    let mut buf_len: u32 = 16 * 1024;
    let mut buf: Vec<u8> = vec![0; buf_len as usize];

    for _ in 0..3 {
        let ret = unsafe {
            GetAdaptersAddresses(
                AF_INET as u32,
                flags,
                std::ptr::null(),
                buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
                &mut buf_len,
            )
        };
        if ret == ERROR_SUCCESS {
            break;
        }
        if ret == ERROR_BUFFER_OVERFLOW {
            buf.resize(buf_len as usize, 0);
            continue;
        }
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut adapter_ptr = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;

    while !adapter_ptr.is_null() {
        let adapter = unsafe { &*adapter_ptr };
        adapter_ptr = adapter.Next;

        if adapter.OperStatus != IfOperStatusUp {
            continue;
        }
        // 排除回环(24)、PPP(23)、虚拟(53)、隧道(131)
        if matches!(adapter.IfType, 23 | 24 | 53 | 131) {
            continue;
        }

        let name = unsafe { wide_ptr_to_string(adapter.FriendlyName) };
        let desc = unsafe { wide_ptr_to_string(adapter.Description) };
        if is_virtual_or_cellular_adapter(&name, &desc) {
            continue;
        }

        let has_gateway = !adapter.FirstGatewayAddress.is_null();
        let mut base_priority: i32 = 0;
        if has_gateway {
            base_priority += 100;
        }
        if adapter.IfType == IF_TYPE_IEEE80211 {
            base_priority += 40;
        } else if adapter.IfType == IF_TYPE_ETHERNET_CSMACD {
            base_priority += 30;
        }
        base_priority -= (adapter.Ipv4Metric.min(5_000)) as i32;

        let mut unicast_ptr = adapter.FirstUnicastAddress;
        while !unicast_ptr.is_null() {
            let unicast = unsafe { &*unicast_ptr };
            unicast_ptr = unicast.Next;

            let sa_ptr = unicast.Address.lpSockaddr;
            if sa_ptr.is_null()
                || (unicast.Address.iSockaddrLength as usize) < std::mem::size_of::<SOCKADDR_IN>()
            {
                continue;
            }
            let sa_family = unsafe { (*sa_ptr).sa_family };
            if sa_family != AF_INET {
                continue;
            }

            let sin = unsafe { &*(sa_ptr as *const SOCKADDR_IN) };
            let ip = Ipv4Addr::from(u32::from_be(unsafe { sin.sin_addr.S_un.S_addr }));
            let prefix_len = unicast.OnLinkPrefixLength;

            if !is_rfc1918_private(ip) || is_clash_fake_ip(ip) || is_carrier_cgnat(ip) {
                continue;
            }
            if !(8..=29).contains(&prefix_len) {
                continue;
            }

            let netmask = netmask_from_prefix(prefix_len);
            let broadcast = compute_broadcast(ip, netmask);
            let ip_bonus = match ip.octets()[0] {
                192 => 20,
                10 => 10,
                _ => 5,
            };

            result.push(LanInterface {
                name: name.clone(),
                description: desc.clone(),
                ip,
                netmask,
                broadcast,
                priority: base_priority + ip_bonus,
            });
        }
    }

    sort_and_dedup_interfaces(result)
}

#[cfg(unix)]
pub fn enumerate_lan_interfaces() -> Vec<LanInterface> {
    use std::ffi::CStr;

    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut head) } != 0 || head.is_null() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut cur = head;

    while !cur.is_null() {
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;

        let flags = ifa.ifa_flags;
        let is_up = (flags & (libc::IFF_UP as libc::c_uint)) != 0;
        let is_running = (flags & (libc::IFF_RUNNING as libc::c_uint)) != 0;
        let is_loopback = (flags & (libc::IFF_LOOPBACK as libc::c_uint)) != 0;
        let is_p2p = (flags & (libc::IFF_POINTOPOINT as libc::c_uint)) != 0;
        let has_bcast = (flags & (libc::IFF_BROADCAST as libc::c_uint)) != 0;

        // 严格剔除未启用、回环以及点对点网卡（蜂窝数据 rmnet/ccmni 与 VPN tun 均带 IFF_POINTOPOINT）
        if !is_up || !is_running || is_loopback || is_p2p || !has_bcast {
            continue;
        }
        if ifa.ifa_name.is_null() || ifa.ifa_addr.is_null() || ifa.ifa_netmask.is_null() {
            continue;
        }

        let name = unsafe { CStr::from_ptr(ifa.ifa_name) }
            .to_string_lossy()
            .into_owned();
        if is_virtual_or_cellular_adapter(&name, "") {
            continue;
        }

        let addr_family = unsafe { (*ifa.ifa_addr).sa_family } as libc::c_int;
        let mask_family = unsafe { (*ifa.ifa_netmask).sa_family } as libc::c_int;
        if addr_family != libc::AF_INET || mask_family != libc::AF_INET {
            continue;
        }

        let sin_addr = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in) };
        let sin_mask = unsafe { &*(ifa.ifa_netmask as *const libc::sockaddr_in) };
        let ip = Ipv4Addr::from(u32::from_be(sin_addr.sin_addr.s_addr));
        let netmask = Ipv4Addr::from(u32::from_be(sin_mask.sin_addr.s_addr));

        if !is_rfc1918_private(ip) || is_clash_fake_ip(ip) || is_carrier_cgnat(ip) {
            continue;
        }
        let Some(prefix_len) = prefix_from_netmask(netmask) else {
            continue;
        };
        if !(8..=29).contains(&prefix_len) {
            continue;
        }

        let low = name.to_ascii_lowercase();
        let mut priority: i32 = if low.starts_with("ap")
            || low.starts_with("swlan")
            || low.starts_with("softap")
            || low.starts_with("wlan")
            || low.starts_with("en")
        {
            80
        } else if low.starts_with("eth") || low.starts_with("rndis") {
            60
        } else {
            20
        };
        priority += match ip.octets()[0] {
            192 => 20,
            10 => 10,
            _ => 5,
        };

        result.push(LanInterface {
            name: name.clone(),
            description: name,
            ip,
            netmask,
            broadcast: compute_broadcast(ip, netmask),
            priority,
        });
    }

    unsafe { libc::freeifaddrs(head) };
    sort_and_dedup_interfaces(result)
}

#[cfg(not(any(windows, unix)))]
pub fn enumerate_lan_interfaces() -> Vec<LanInterface> {
    Vec::new()
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
    let lan_ifaces = enumerate_lan_interfaces();
    if lan_ifaces.is_empty() {
        return Err(
            "未检测到可用的 Wi-Fi 或个人热点局域网网卡，请先连接同一 Wi-Fi 或开启个人热点（已避免占用移动流量）"
                .to_string(),
        );
    }
    start_server_with_interfaces(db, lan_ifaces, true, on_received).await
}

async fn start_server_with_interfaces<F>(
    db: Arc<Mutex<Connection>>,
    lan_ifaces: Vec<LanInterface>,
    enable_discovery: bool,
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

    let primary_ip = lan_ifaces[0].ip.to_string();
    let address = format!("{primary_ip}:{port}");
    let addresses: Vec<String> = lan_ifaces
        .iter()
        .map(|iface| format!("{}:{port}", iface.ip))
        .collect();
    let pin = generate_pin();
    let session_id = generate_session_id();
    let device_name = local_device_name();
    let platform = local_platform().to_string();

    let info = LanServerInfo {
        ip: primary_ip,
        port,
        address,
        addresses,
        pin: pin.clone(),
        device_name: device_name.clone(),
        platform: platform.clone(),
    };

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let activity_notify = Arc::new(Notify::new());
    let failed_attempts = Arc::new(AtomicU8::new(0));
    let callback = Arc::new(on_received);

    if enable_discovery {
        tokio::spawn(run_discovery_responder(
            lan_ifaces,
            session_id.clone(),
            device_name.clone(),
            platform.clone(),
            port,
            shutdown_rx.clone(),
        ));
    }

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
                        Ok((stream, peer_addr)) => {
                            if !is_allowed_lan_peer_ip(peer_addr.ip()) {
                                drop(stream);
                                continue;
                            }
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

struct NicDiscoverySender {
    iface: LanInterface,
    socket: UdpSocket,
    beacon_bytes: Vec<u8>,
}

async fn run_discovery_responder(
    lan_ifaces: Vec<LanInterface>,
    session_id: String,
    device_name: String,
    platform: String,
    port: u16,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut senders = Vec::with_capacity(lan_ifaces.len());
    for iface in lan_ifaces {
        let Ok(sock) = UdpSocket::bind(SocketAddr::from((iface.ip, 0))).await else {
            continue;
        };
        let _ = sock.set_broadcast(true);
        let beacon = DiscoveryBeacon {
            app: "trace".to_string(),
            session_id: session_id.clone(),
            device_name: device_name.clone(),
            platform: platform.clone(),
            ip: iface.ip.to_string(),
            port,
        };
        let Ok(beacon_bytes) = serde_json::to_vec(&beacon) else {
            continue;
        };
        senders.push(NicDiscoverySender {
            iface,
            socket: sock,
            beacon_bytes,
        });
    }

    if senders.is_empty() {
        return;
    }

    let listener_sock = UdpSocket::bind(("0.0.0.0", SYNC_DISCOVERY_PORT)).await.ok();
    let mut recv_buf = [0u8; 512];
    let mut tick = tokio::time::interval(Duration::from_millis(2500));

    loop {
        if let Some(sock) = listener_sock.as_ref() {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
                _ = tick.tick() => {
                    for sender in &senders {
                        let target = SocketAddr::from((sender.iface.broadcast, SYNC_DISCOVERY_PORT));
                        let _ = sender.socket.send_to(&sender.beacon_bytes, target).await;
                    }
                }
                recv_res = sock.recv_from(&mut recv_buf) => {
                    if let Ok((len, src)) = recv_res {
                        if !is_allowed_lan_peer_ip(src.ip()) {
                            continue;
                        }
                        if let Ok(probe) = serde_json::from_slice::<DiscoveryProbe>(&recv_buf[..len]) {
                            if probe.app == "trace" && probe.probe == 1 {
                                reply_matching_nic(&senders, src).await;
                            }
                        }
                    }
                }
            }
        } else {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
                _ = tick.tick() => {
                    for sender in &senders {
                        let target = SocketAddr::from((sender.iface.broadcast, SYNC_DISCOVERY_PORT));
                        let _ = sender.socket.send_to(&sender.beacon_bytes, target).await;
                    }
                }
            }
        }
    }
}

async fn reply_matching_nic(senders: &[NicDiscoverySender], src: SocketAddr) {
    if let IpAddr::V4(src_v4) = src.ip() {
        if let Some(matched) = senders
            .iter()
            .find(|s| same_subnet(src_v4, s.iface.ip, s.iface.netmask))
        {
            let _ = matched.socket.send_to(&matched.beacon_bytes, src).await;
            return;
        }
    }
    if let Some(first) = senders.first() {
        let _ = first.socket.send_to(&first.beacon_bytes, src).await;
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

fn parse_beacon_if_valid(
    raw: &[u8],
    src_addr: SocketAddr,
    own_session_id: Option<&str>,
) -> Option<LanPeer> {
    if !is_allowed_lan_peer_ip(src_addr.ip()) {
        return None;
    }
    let beacon = serde_json::from_slice::<DiscoveryBeacon>(raw).ok()?;
    if beacon.app != "trace" || beacon.port == 0 {
        return None;
    }
    if own_session_id.is_some_and(|sid| sid == beacon.session_id) {
        return None;
    }
    let peer_ip = match src_addr.ip() {
        IpAddr::V4(v4) if is_rfc1918_private(v4) => v4.to_string(),
        _ => beacon.ip,
    };
    let parsed_ip: Ipv4Addr = peer_ip.parse().ok()?;
    if !is_rfc1918_private(parsed_ip) {
        return None;
    }
    let address = format!("{peer_ip}:{}", beacon.port);
    Some(LanPeer {
        device_name: beacon.device_name,
        platform: beacon.platform,
        ip: peer_ip,
        port: beacon.port,
        address,
    })
}

pub async fn discover_peers(
    timeout_ms: u64,
    own_session_id: Option<&str>,
) -> Result<Vec<LanPeer>, String> {
    let lan_ifaces = enumerate_lan_interfaces();
    if lan_ifaces.is_empty() {
        // 不触碰蜂窝流量或虚拟 TUN 网卡
        return Ok(Vec::new());
    }

    let wait_ms = timeout_ms.clamp(500, 5000);
    let (tx, mut rx) = mpsc::channel::<LanPeer>(32);
    let (done_tx, done_rx) = watch::channel(false);
    let own_sid = own_session_id.map(str::to_string);

    // 1) 为每个真实局域网/热点网卡创建独立绑定在 iface.ip 的 UDP socket，只向对应子网广播发探测
    for iface in lan_ifaces {
        let Ok(sock) = UdpSocket::bind(SocketAddr::from((iface.ip, 0))).await else {
            continue;
        };
        let _ = sock.set_broadcast(true);
        let probe = br#"{"app":"trace","probe":1}"#;
        let bcast_target = SocketAddr::from((iface.broadcast, SYNC_DISCOVERY_PORT));
        let _ = sock.send_to(probe, bcast_target).await;

        let tx_clone = tx.clone();
        let mut done_clone = done_rx.clone();
        let sid_clone = own_sid.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            let second_tick = tokio::time::sleep(Duration::from_millis(wait_ms / 2));
            tokio::pin!(second_tick);
            let mut sent_second = false;

            loop {
                tokio::select! {
                    _ = done_clone.changed() => {
                        if *done_clone.borrow() {
                            break;
                        }
                    }
                    _ = &mut second_tick, if !sent_second => {
                        sent_second = true;
                        let _ = sock.send_to(probe, bcast_target).await;
                    }
                    recv_res = sock.recv_from(&mut buf) => {
                        if let Ok((len, src_addr)) = recv_res {
                            if let Some(peer) = parse_beacon_if_valid(&buf[..len], src_addr, sid_clone.as_deref()) {
                                let _ = tx_clone.send(peer).await;
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
        });
    }

    // 2) 若 17581 端口空闲，同步监听对方主动发出的周期信标
    if let Ok(passive_sock) = UdpSocket::bind(("0.0.0.0", SYNC_DISCOVERY_PORT)).await {
        let tx_clone = tx.clone();
        let mut done_clone = done_rx.clone();
        let sid_clone = own_sid;
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            loop {
                tokio::select! {
                    _ = done_clone.changed() => {
                        if *done_clone.borrow() {
                            break;
                        }
                    }
                    recv_res = passive_sock.recv_from(&mut buf) => {
                        if let Ok((len, src_addr)) = recv_res {
                            if let Some(peer) = parse_beacon_if_valid(&buf[..len], src_addr, sid_clone.as_deref()) {
                                let _ = tx_clone.send(peer).await;
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
        });
    }

    drop(tx);

    let mut peers: HashMap<String, LanPeer> = HashMap::new();
    let deadline = Instant::now() + Duration::from_millis(wait_ms);
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        match tokio::time::timeout(deadline - now, rx.recv()).await {
            Ok(Some(peer)) => {
                peers.insert(peer.address.clone(), peer);
            }
            Ok(None) | Err(_) => break,
        }
    }
    let _ = done_tx.send(true);

    let mut list: Vec<LanPeer> = peers.into_values().collect();
    list.sort_by(|a, b| a.device_name.cmp(&b.device_name).then(a.address.cmp(&b.address)));
    Ok(list)
}

fn validate_sync_target(normalized_addr: &str) -> Result<Option<Ipv4Addr>, String> {
    let Some((host, _)) = normalized_addr.rsplit_once(':') else {
        return Err("连接地址格式无效".to_string());
    };
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        if is_clash_fake_ip(ip) {
            return Err(format!(
                "地址 {ip} 属于 Clash/Mihomo 虚拟代理网卡，不是真实局域网 IP，请填写对方设备显示的 Wi-Fi 或热点地址"
            ));
        }
        if !is_rfc1918_private(ip) && !ip.is_loopback() {
            return Err(format!(
                "为防止误走移动流量，仅支持连接局域网私有 IP（192.168.x.x / 10.x.x.x / 172.16-31.x.x），已拦截目标：{ip}"
            ));
        }
        return Ok(Some(ip));
    }
    if host.eq_ignore_ascii_case("localhost") {
        return Ok(Some(Ipv4Addr::LOCALHOST));
    }
    Err("请直接填写对方设备的局域网 IP:端口（例如 192.168.81.57:17580）".to_string())
}

pub async fn sync_with_peer(
    db: Arc<Mutex<Connection>>,
    address: &str,
    pin: &str,
) -> Result<LanSyncOutcome, String> {
    let normalized_addr = normalize_peer_address(address)?;
    let target_ip = validate_sync_target(&normalized_addr)?;
    let clean_pin = pin.trim();
    if clean_pin.len() != 6 || !clean_pin.bytes().all(|b| b.is_ascii_digit()) {
        return Err("请输入 6 位数字配对码".to_string());
    }

    let mut local_bind_ip: Option<IpAddr> = None;
    if let Some(ipv4) = target_ip {
        if !ipv4.is_loopback() {
            let lan_ifaces = enumerate_lan_interfaces();
            if lan_ifaces.is_empty() {
                return Err(
                    "未检测到可用的 Wi-Fi 或个人热点局域网网卡，已阻止同步请求以避免走移动流量"
                        .to_string(),
                );
            }
            if let Some(matched) = lan_ifaces
                .iter()
                .find(|iface| same_subnet(ipv4, iface.ip, iface.netmask))
            {
                local_bind_ip = Some(IpAddr::V4(matched.ip));
            } else if cfg!(target_os = "android") {
                let ips: Vec<String> = lan_ifaces.iter().map(|i| i.ip.to_string()).collect();
                return Err(format!(
                    "目标地址 {ipv4} 与本机当前局域网/热点网卡（{}）不在同一网段，已拦截以防误走手机流量。请核对对方显示的真实局域网 IP",
                    ips.join("、")
                ));
            }
        }
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

    let mut client_builder = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS));
    if let Some(bind_ip) = local_bind_ip {
        client_builder = client_builder.local_address(bind_ip);
    }
    let client = client_builder
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
                "连接对方设备超时：请确认双方处于同一局域网/热点，系统防火墙已放行；若手机开启了代理/VPN，请先配置绕过局域网或暂时关闭代理".to_string()
            } else if e.is_connect() {
                "无法连接到对方设备：请确认对方已开启局域网快传、地址端口填写正确，且手机端未因代理/VPN 拦截局域网直连".to_string()
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

    fn loopback_test_interface() -> Vec<LanInterface> {
        vec![LanInterface {
            name: "Loopback".to_string(),
            description: "Loopback".to_string(),
            ip: Ipv4Addr::LOCALHOST,
            netmask: Ipv4Addr::new(255, 0, 0, 0),
            broadcast: Ipv4Addr::new(127, 255, 255, 255),
            priority: 1,
        }]
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

    #[test]
    fn filters_virtual_vpn_and_cellular_adapters() {
        assert!(is_virtual_or_cellular_adapter("Mihomo", "Meta Tunnel"));
        assert!(is_virtual_or_cellular_adapter(
            "VMware Network Adapter VMnet8",
            "VMware Virtual Ethernet Adapter"
        ));
        assert!(is_virtual_or_cellular_adapter(
            "VirtualBox Host-Only Network",
            "VirtualBox Host-Only Ethernet Adapter"
        ));
        assert!(is_virtual_or_cellular_adapter("rmnet_data0", ""));
        assert!(is_virtual_or_cellular_adapter("tun0", ""));
        assert!(is_virtual_or_cellular_adapter("ccmni1", ""));
        assert!(!is_virtual_or_cellular_adapter(
            "WLAN",
            "MediaTek Wi-Fi 6 MT7921 Wireless LAN Card"
        ));
        assert!(!is_virtual_or_cellular_adapter("wlan0", ""));
        assert!(!is_virtual_or_cellular_adapter("ap0", ""));
        assert!(!is_virtual_or_cellular_adapter("swlan0", ""));
    }

    #[test]
    fn classifies_lan_vs_fake_ip_and_cgnat_and_broadcast() {
        assert!(is_rfc1918_private(Ipv4Addr::new(192, 168, 81, 57)));
        assert!(is_rfc1918_private(Ipv4Addr::new(10, 0, 0, 5)));
        assert!(is_rfc1918_private(Ipv4Addr::new(172, 16, 1, 2)));
        assert!(!is_rfc1918_private(Ipv4Addr::new(198, 18, 0, 1)));
        assert!(is_clash_fake_ip(Ipv4Addr::new(198, 18, 0, 1)));
        assert!(is_clash_fake_ip(Ipv4Addr::new(198, 19, 255, 254)));
        assert!(is_carrier_cgnat(Ipv4Addr::new(100, 64, 0, 1)));
        assert_eq!(
            prefix_from_netmask(netmask_from_prefix(24)),
            Some(24)
        );
        assert_eq!(
            compute_broadcast(
                Ipv4Addr::new(192, 168, 81, 57),
                netmask_from_prefix(24)
            ),
            Ipv4Addr::new(192, 168, 81, 255)
        );
        assert!(same_subnet(
            Ipv4Addr::new(192, 168, 81, 57),
            Ipv4Addr::new(192, 168, 81, 63),
            netmask_from_prefix(24)
        ));
        assert!(!same_subnet(
            Ipv4Addr::new(192, 168, 81, 57),
            Ipv4Addr::new(198, 18, 0, 1),
            netmask_from_prefix(24)
        ));
    }

    #[tokio::test]
    async fn rejects_clash_fake_ip_and_public_ip_before_sending() {
        let db = mem_db_arc();
        let err_fake = sync_with_peer(Arc::clone(&db), "198.18.0.1:17580", "123456")
            .await
            .unwrap_err();
        assert!(err_fake.contains("Mihomo"), "unexpected err: {err_fake}");

        let err_pub = sync_with_peer(Arc::clone(&db), "8.8.8.8:17580", "123456")
            .await
            .unwrap_err();
        assert!(err_pub.contains("局域网私有 IP"), "unexpected err: {err_pub}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_enumeration_excludes_mihomo_and_virtual_adapters() {
        let ifaces = enumerate_lan_interfaces();
        assert!(!ifaces.is_empty(), "expected at least one real LAN adapter");
        for iface in &ifaces {
            println!(
                "DETECTED_IFACE: {} ({}) ip={} mask={} bcast={} prio={}",
                iface.name, iface.description, iface.ip, iface.netmask, iface.broadcast, iface.priority
            );
            assert!(
                is_rfc1918_private(iface.ip),
                "unexpected non-RFC1918 ip: {}",
                iface.ip
            );
            assert!(
                !is_clash_fake_ip(iface.ip),
                "Clash/Mihomo IP leaked: {}",
                iface.ip
            );
            assert!(
                !is_virtual_or_cellular_adapter(&iface.name, &iface.description),
                "Virtual adapter leaked: {} ({})",
                iface.name,
                iface.description
            );
        }
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
        let (handle, info) = start_server_with_interfaces(
            Arc::clone(&db_server),
            loopback_test_interface(),
            false,
            move |evt| {
                events_clone.lock().unwrap().push(evt);
            },
        )
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

        let (handle, info) = start_server_with_interfaces(
            Arc::clone(&db_server),
            loopback_test_interface(),
            false,
            |_| {},
        )
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
