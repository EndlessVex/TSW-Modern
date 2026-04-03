use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicU32, Ordering};
use std::io::Write;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::Mutex;

const REAL_CDN: &str = "update.secretworld.com";
const REAL_PORT: u16 = 80;

type LogFile = Arc<Mutex<std::fs::File>>;

async fn log_msg(log: &LogFile, msg: &str) {
    println!("{}", msg);
    if let Ok(mut f) = log.lock().await {
        let _ = writeln!(f, "{}", msg);
        let _ = f.flush();
    }
}

#[tokio::main]
async fn main() {
    let log_file = std::fs::File::create("intercept_log.txt").expect("create log file");
    let log: LogFile = Arc::new(Mutex::new(log_file));

    log_msg(&log, "=== ClientPatcher Traffic Interceptor ===").await;
    log_msg(&log, "Logging to intercept_log.txt").await;
    log_msg(&log, "").await;

    let http_files = Arc::new(AtomicU32::new(0));
    let http_bytes = Arc::new(AtomicU64::new(0));

    let hf = http_files.clone();
    let hb = http_bytes.clone();
    let log2 = log.clone();
    let http_task = tokio::spawn(async move {
        run_http_proxy(hf, hb, log2).await;
    });

    let log3 = log.clone();
    let udp_task = tokio::spawn(async move {
        run_udp_listener(log3).await;
    });

    log_msg(&log, "HTTP proxy listening on port 8888").await;
    log_msg(&log, "UDP listener on port 6969").await;
    log_msg(&log, "").await;
    log_msg(&log, "Edit LocalConfig.xml:").await;
    log_msg(&log, "  <HttpPatchAddr>http://localhost:8888/tswupm</HttpPatchAddr>").await;
    log_msg(&log, "").await;
    log_msg(&log, "Then run ClientPatcher.exe and watch the traffic.").await;
    log_msg(&log, &format!("{:-<120}", "")).await;

    let _ = tokio::join!(http_task, udp_task);
}

async fn run_http_proxy(files: Arc<AtomicU32>, bytes: Arc<AtomicU64>, log: LogFile) {
    let listener = TcpListener::bind("0.0.0.0:8888").await.expect("bind 8888");

    loop {
        let (client, _addr) = listener.accept().await.expect("accept");
        let files = files.clone();
        let bytes = bytes.clone();
        let log = log.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_http_connection(client, files, bytes, log.clone()).await {
                log_msg(&log, &format!("  [HTTP ERROR] {}", e)).await;
            }
        });
    }
}

async fn handle_http_connection(
    mut client: TcpStream,
    files: Arc<AtomicU32>,
    bytes: Arc<AtomicU64>,
    log: LogFile,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut buf = vec![0u8; 16384];
    let n = client.read(&mut buf).await?;
    if n == 0 { return Ok(()); }

    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    let method = parts.get(0).unwrap_or(&"?");
    let path = parts.get(1).unwrap_or(&"?");

    // Log ALL headers
    let headers: Vec<&str> = request.lines().skip(1).take_while(|l| !l.is_empty()).collect();

    let file_num = files.fetch_add(1, Ordering::Relaxed) + 1;
    let timestamp = chrono_lite();
    
    let mut log_lines = format!("[{timestamp}] #{file_num} {method} {path}");
    for h in &headers {
        log_lines.push_str(&format!("\n  {h}"));
    }

    // Forward to real CDN
    let mut upstream = TcpStream::connect((REAL_CDN, REAL_PORT)).await?;
    let modified_request = request.replace("localhost:8888", REAL_CDN)
        .replace("127.0.0.1:8888", REAL_CDN);
    upstream.write_all(modified_request.as_bytes()).await?;

    // Read response
    let mut resp_buf = vec![0u8; 65536];
    let resp_n = upstream.read(&mut resp_buf).await?;
    if resp_n == 0 {
        log_lines.push_str("\n  → (no response)");
        log_msg(&log, &log_lines).await;
        return Ok(());
    }

    let resp_str = String::from_utf8_lossy(&resp_buf[..resp_n]);
    let status_line = resp_str.lines().next().unwrap_or("?");
    
    let content_length: u64 = resp_str.lines()
        .find(|l| l.to_lowercase().starts_with("content-length"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);

    // Log response headers too
    let resp_headers: Vec<&str> = resp_str.lines().take_while(|l| !l.is_empty()).collect();
    log_lines.push_str(&format!("\n  → {status_line} ({content_length} bytes)"));
    for rh in &resp_headers[1..] {
        log_lines.push_str(&format!("\n  ← {rh}"));
    }

    // Forward response to client
    client.write_all(&resp_buf[..resp_n]).await?;
    let mut total_forwarded = resp_n as u64;
    loop {
        let n = upstream.read(&mut resp_buf).await?;
        if n == 0 { break; }
        client.write_all(&resp_buf[..n]).await?;
        total_forwarded += n as u64;
    }

    bytes.fetch_add(total_forwarded, Ordering::Relaxed);
    
    let total_files = files.load(Ordering::Relaxed);
    let total_bytes = bytes.load(Ordering::Relaxed);
    if total_files % 50 == 0 {
        log_lines.push_str(&format!("\n  --- Progress: {total_files} files, {:.1} MB total ---",
            total_bytes as f64 / 1_048_576.0));
    }

    log_msg(&log, &log_lines).await;
    Ok(())
}

async fn run_udp_listener(log: LogFile) {
    for port in [69u16, 6969, 4069] {
        let sock = match UdpSocket::bind(format!("0.0.0.0:{}", port)).await {
            Ok(s) => {
                log_msg(&log, &format!("UDP listener on port {port}")).await;
                s
            }
            Err(e) => {
                log_msg(&log, &format!("  Could not bind UDP port {port}: {e}")).await;
                continue;
            }
        };

        let log2 = log.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                match sock.recv_from(&mut buf).await {
                    Ok((n, addr)) => {
                        let hex: String = buf[..n.min(64)].iter()
                            .map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ");
                        let ascii: String = buf[..n.min(64)].iter()
                            .map(|&b| if b >= 32 && b < 127 { b as char } else { '.' })
                            .collect();
                        log_msg(&log2, &format!("[UDP:{port}] {addr} → {n} bytes: {hex}")).await;
                        log_msg(&log2, &format!("  ASCII: {ascii}")).await;
                    }
                    Err(e) => {
                        log_msg(&log2, &format!("[UDP:{port}] Error: {e}")).await;
                        break;
                    }
                }
            }
        });
    }

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}

fn chrono_lite() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs() % 86400;
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}
