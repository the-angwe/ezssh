//! `ezssh-testsrv --root DIR [--port 2222] [--user ezssh] [--password ezssh] [--home /] [--delay-ms 0]`
//!
//! Serves DIR as remote `/` over SFTP on 127.0.0.1, with an echo shell,
//! so the file pane can be tried without a real sshd.

use std::net::SocketAddr;
use std::time::Duration;

use ezssh_testsrv::{Config, TestServer};

fn usage() -> ! {
    eprintln!(
        "usage: ezssh-testsrv --root DIR [--port 2222] [--user ezssh] [--password ezssh] [--home /] [--delay-ms 0]"
    );
    std::process::exit(2);
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ezssh_testsrv=info".into()),
        )
        .init();
    let mut root = None;
    let (mut port, mut user, mut password, mut home, mut delay) = (
        2222u16,
        "ezssh".to_string(),
        "ezssh".to_string(),
        "/".to_string(),
        0u64,
    );
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "--root" => root = Some(val()),
            "--port" => port = val().parse().unwrap_or_else(|_| usage()),
            "--user" => user = val(),
            "--password" => password = val(),
            "--home" => home = val(),
            "--delay-ms" => delay = val().parse().unwrap_or_else(|_| usage()),
            _ => usage(),
        }
    }
    let Some(root) = root else { usage() };
    let mut cfg = Config::new(root, &user, &password);
    cfg.home = home;
    cfg.op_delay = Duration::from_millis(delay);
    let srv = TestServer::start(cfg, SocketAddr::from(([127, 0, 0, 1], port))).await?;
    println!(
        "ezssh-testsrv on {} as {user} / {password}, host key {}",
        srv.addr, srv.fingerprint
    );
    tokio::signal::ctrl_c().await?;
    Ok(())
}
