use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::{RwLock, mpsc};

use russh::client::Handle as RusshHandle;

use crate::ssh::client::ClientHandler;
use crate::ssh::forwards::{ForwardKind, ForwardSpec, bind_socket};

/// A live SSH session. The write half is an mpsc sender: the command layer
/// enqueues keystrokes and the per-connection writer task drains them to the
/// russh channel. The russh `Handle` is wrapped in `Arc<...>` (no Mutex) —
/// every method we call on it (`channel_open_session`,
/// `channel_open_direct_tcpip`, `tcpip_forward`, `cancel_tcpip_forward`,
/// `disconnect`) takes `&self`, so concurrent access is safe and a Mutex
/// would only serialize SSH channel opens behind one another.
pub struct Connection {
    pub id: u64,
    /// The saved session this connection was opened from, and its name at
    /// connect time. Lets a port conflict name the tab holding the port.
    pub session_id: i64,
    pub session_name: String,
    pub stdin: mpsc::UnboundedSender<ConnectionInput>,
    /// Shared russh client handle. The driver task holds a clone for the
    /// lifetime of the session so the Close branch can `disconnect(..)`; the
    /// SFTP commands clone this Arc to open a second session channel.
    pub ssh_handle: Arc<RusshHandle<ClientHandler>>,
    /// When `Some(display)`, this session acquired the X server on that
    /// display and must release it on disconnect. The driver task reads
    /// this at teardown. Tagged `allow(dead_code)` because the only
    /// current reader is the closure captured by the driver — Rust's
    /// dead-code lint can't see through that.
    #[allow(dead_code)]
    pub x11_display: Option<u8>,
    /// Per-connection port-forwarding runtime. Created empty at insert
    /// time; populated by `api::forwards::forward_start` and by
    /// the auto-start scan in `connect_impl`.
    pub forwards: std::sync::Arc<crate::ssh::forwards::Forwards>,
}

pub enum ConnectionInput {
    Bytes(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Close,
}

#[derive(Default)]
pub struct ConnectionRegistry {
    next_id: AtomicU64,
    inner: RwLock<HashMap<u64, Arc<Connection>>>,
}

impl ConnectionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alloc_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub async fn insert(&self, conn: Connection) {
        self.inner.write().await.insert(conn.id, Arc::new(conn));
    }

    pub async fn get(&self, id: u64) -> Option<Arc<Connection>> {
        self.inner.read().await.get(&id).cloned()
    }

    /// Snapshot of every live connection. Used by command paths that
    /// need to touch every tab's per-connection forwards state — e.g.
    /// updating or deleting a persistent forward that's currently
    /// running in one or more tabs.
    pub async fn list_all(&self) -> Vec<Arc<Connection>> {
        self.inner.read().await.values().cloned().collect()
    }

    pub async fn write(&self, id: u64, bytes: Vec<u8>) -> bool {
        if let Some(conn) = self.get(id).await {
            conn.stdin.send(ConnectionInput::Bytes(bytes)).is_ok()
        } else {
            false
        }
    }

    pub async fn resize(&self, id: u64, cols: u16, rows: u16) -> bool {
        if let Some(conn) = self.get(id).await {
            conn.stdin
                .send(ConnectionInput::Resize { cols, rows })
                .is_ok()
        } else {
            false
        }
    }

    /// The connection whose running forward holds the local address `spec`
    /// would bind (Local / Dynamic only), with that forward. Used to name
    /// the holder in a port-conflict message. Several holders cannot exist:
    /// the first one bound the port.
    pub async fn local_bind_holder(
        &self,
        spec: &ForwardSpec,
    ) -> Option<(Arc<Connection>, ForwardSpec)> {
        if spec.kind == ForwardKind::Remote {
            return None;
        }
        for conn in self.list_all().await {
            for rf in conn.forwards.by_id.read().await.values() {
                if rf.spec.kind != ForwardKind::Remote && binds_overlap(&rf.spec, spec) {
                    return Some((conn.clone(), rf.spec.clone()));
                }
            }
        }
        None
    }

    pub async fn close(&self, id: u64) {
        let conn = self.inner.write().await.remove(&id);
        if let Some(c) = conn {
            // Stop forwards before signalling the driver to drop the
            // russh handle — Remote forwards need the handle for
            // cancel_tcpip_forward at teardown.
            c.forwards.stop_all().await;
            let _ = c.stdin.send(ConnectionInput::Close);
        }
    }
}

/// Two local binds collide: same port, and the same address or either one
/// on the unspecified address (`0.0.0.0` / `::` take every interface).
/// Unparseable addresses never collide (their bind failed anyway).
fn binds_overlap(a: &ForwardSpec, b: &ForwardSpec) -> bool {
    if a.bind_port != b.bind_port {
        return false;
    }
    match (
        bind_socket(&a.bind_addr, a.bind_port),
        bind_socket(&b.bind_addr, b.bind_port),
    ) {
        (Ok(x), Ok(y)) => x.ip() == y.ip() || x.ip().is_unspecified() || y.ip().is_unspecified(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(addr: &str, port: u16) -> ForwardSpec {
        ForwardSpec {
            name: String::new(),
            kind: ForwardKind::Local,
            bind_addr: addr.into(),
            bind_port: port,
            dest_addr: "127.0.0.1".into(),
            dest_port: 80,
        }
    }

    #[test]
    fn overlap_rules() {
        assert!(binds_overlap(
            &local("127.0.0.1", 8000),
            &local("127.0.0.1", 8000)
        ));
        assert!(binds_overlap(
            &local("localhost", 8000),
            &local("127.0.0.1", 8000)
        ));
        assert!(binds_overlap(
            &local("0.0.0.0", 8000),
            &local("127.0.0.1", 8000)
        ));
        assert!(binds_overlap(&local("::1", 8000), &local("::", 8000)));
        assert!(!binds_overlap(
            &local("127.0.0.1", 8000),
            &local("127.0.0.1", 8001)
        ));
        assert!(!binds_overlap(
            &local("127.0.0.1", 8000),
            &local("127.0.0.2", 8000)
        ));
        assert!(!binds_overlap(
            &local("bogus", 8000),
            &local("127.0.0.1", 8000)
        ));
    }
}
