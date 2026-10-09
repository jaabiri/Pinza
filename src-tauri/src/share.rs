//! Send to phone: serves one file on the local network, behind a random
//! link that expires, and draws the link as a QR code. Nothing goes through
//! the internet.

use qrcode::render::svg;
use qrcode::QrCode;
use rand::Rng;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const LIFETIME: Duration = Duration::from_secs(10 * 60);

type Shares = Arc<Mutex<HashMap<String, (PathBuf, Instant)>>>;

struct Server {
    port: u16,
    shares: Shares,
}

static SERVER: OnceLock<Result<Server, String>> = OnceLock::new();

pub struct Shared {
    pub url: String,
    pub svg: String,
    pub minutes: u64,
}

pub fn share(path: &Path) -> Result<Shared, String> {
    let server = SERVER.get_or_init(start).as_ref().map_err(Clone::clone)?;
    let ip = lan_ip().ok_or("Not connected to a network")?;
    let token: String = rand::rng().sample_iter(rand::distr::Alphanumeric).take(22).map(char::from).collect();
    {
        let mut shares = server.shares.lock().unwrap();
        shares.retain(|_, (_, until)| *until > Instant::now());
        shares.insert(token.clone(), (path.to_path_buf(), Instant::now() + LIFETIME));
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let url = format!("http://{ip}:{}/{token}/{}", server.port, encode(&name));
    let code = QrCode::new(url.as_bytes()).map_err(|e| e.to_string())?;
    let svg = code
        .render::<svg::Color>()
        .min_dimensions(132, 132)
        .quiet_zone(false)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build();
    Ok(Shared { url, svg, minutes: LIFETIME.as_secs() / 60 })
}

fn start() -> Result<Server, String> {
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let shares: Shares = Arc::default();
    let served = shares.clone();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let shares = served.clone();
            thread::spawn(move || {
                let _ = handle(stream, &shares);
            });
        }
    });
    log::info!("Sharing to phones on port {port}");
    Ok(Server { port, shares })
}

fn handle(mut stream: TcpStream, shares: &Shares) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf)?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let mut parts = request.lines().next().unwrap_or("").split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let token = target.trim_start_matches('/').split('/').next().unwrap_or("");
    let found = {
        let shares = shares.lock().unwrap();
        shares.get(token).filter(|(_, until)| *until > Instant::now()).map(|(p, _)| p.clone())
    };
    let Some(path) = found.filter(|_| method == "GET" || method == "HEAD") else {
        let body = "This link has expired.";
        write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())?;
        return Ok(());
    };
    let data = std::fs::read(&path)?;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        content_type(&path),
        data.len()
    )?;
    if method == "GET" {
        stream.write_all(&data)?;
    }
    Ok(())
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("mp4" | "m4v") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
}

/// The address other devices on the network can reach. Connecting a UDP
/// socket sends nothing; it only picks the outgoing interface.
fn lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(10, 254, 254, 254), 1)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
