//! A tiny HTTP/1.1 file server for updater tests: fixed bodies by path,
//! 404 otherwise. A path under `/short/` announces one byte more than it
//! sends (a cut-off download).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serve `files` (path → body) on 127.0.0.1; returns `http://127.0.0.1:PORT`.
/// Runs until the runtime ends.
pub async fn serve(files: HashMap<String, Vec<u8>>) -> std::io::Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let files = Arc::new(files);
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("testsrv http: accept: {e}");
                    return;
                }
            };
            let files = files.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let mut got = 0;
                while !buf[..got].windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut buf[got..]).await {
                        Ok(0) => return,
                        Ok(n) => got += n,
                        Err(e) => {
                            tracing::debug!("testsrv http: read: {e}");
                            return;
                        }
                    }
                    if got == buf.len() {
                        return;
                    }
                }
                let head = String::from_utf8_lossy(&buf[..got]).to_string();
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (short, key) = match path.strip_prefix("/short") {
                    Some(rest) => (true, rest.to_string()),
                    None => (false, path),
                };
                tracing::debug!(path = %key, short, "testsrv http: GET");
                let resp = match files.get(&key) {
                    Some(body) => {
                        let len = body.len() + usize::from(short);
                        let mut r = format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n").into_bytes();
                        r.extend_from_slice(body);
                        r
                    }
                    None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                };
                if let Err(e) = sock.write_all(&resp).await {
                    tracing::debug!("testsrv http: write: {e}");
                }
                let _ = sock.shutdown().await;
            });
        }
    });
    Ok(format!("http://{addr}"))
}
