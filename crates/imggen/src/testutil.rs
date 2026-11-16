//! 测试用的单次响应 HTTP 服务器

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// 接收一个完整请求（头 + Content-Length 指定的 body），回一个固定响应；返回 (地址, 收到的原始请求)
pub async fn serve_once(status: &'static str, body: &'static str) -> (String, tokio::sync::oneshot::Receiver<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut data = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let n = sock.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            data.extend_from_slice(&chunk[..n]);
            let t = String::from_utf8_lossy(&data);
            if let Some(end) = t.find("\r\n\r\n") {
                let len = t[..end]
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                    .unwrap_or(0);
                if data.len() >= end + 4 + len {
                    break;
                }
            }
        }
        let _ = tx.send(String::from_utf8_lossy(&data).to_string());
        let head = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        let _ = sock.write_all(head.as_bytes()).await;
        let _ = sock.write_all(body.as_bytes()).await;
    });
    (addr, rx)
}
