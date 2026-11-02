//! The bridge between the UI thread and `ezssh-core`.
//!
//! A tokio runtime on background threads owns the [`AppState`] and every
//! connection. The UI calls [`Core::call`] with an `api::*` future and a
//! continuation; when the future finishes, the continuation is queued and
//! run on the UI thread with `&mut App` at the start of the next frame
//! ([`Core::take_jobs`]).

use std::future::Future;
use std::path::Path;
use std::sync::{Arc, mpsc};

use anyhow::Context as _;
use ezssh_core::{AppState, Event};

use crate::app::App;

/// A finished call's continuation.
pub type Job = Box<dyn FnOnce(&mut App) + Send>;

pub struct Core {
    rt: tokio::runtime::Runtime,
    pub state: Arc<AppState>,
    jobs_tx: mpsc::Sender<Job>,
    jobs_rx: mpsc::Receiver<Job>,
    ctx: egui::Context,
}

impl Core {
    /// Start the runtime and open the database at `db_path`. Core events
    /// (connection closed, …) arrive on the returned receiver; each one
    /// wakes the UI.
    pub fn open(ctx: &egui::Context, db_path: &Path) -> anyhow::Result<(Self, mpsc::Receiver<Event>)> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("ezssh-core")
            .build()
            .context("starting the tokio runtime")?;
        let wake = ctx.clone();
        let (events, events_rx) = ezssh_core::events::channel(move || wake.request_repaint());
        if let Some(dir) = db_path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let state = rt
            .block_on(AppState::open(db_path, events))
            .with_context(|| format!("opening the database {}", db_path.display()))?;
        let (jobs_tx, jobs_rx) = mpsc::channel();
        Ok((Self { rt, state: Arc::new(state), jobs_tx, jobs_rx, ctx: ctx.clone() }, events_rx))
    }

    /// Run `f(state)` on the runtime; `then(app, result)` runs on the UI
    /// thread once it is done.
    pub fn call<F, Fut, T>(&self, what: &'static str, f: F, then: impl FnOnce(&mut App, T) + Send + 'static)
    where
        F: FnOnce(Arc<AppState>) -> Fut,
        Fut: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        tracing::trace!(what, "core.call");
        let fut = f(self.state.clone());
        let tx = self.jobs_tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn(async move {
            let out = fut.await;
            tracing::trace!(what, "core.call done");
            if tx.send(Box::new(move |app: &mut App| then(app, out))).is_err() {
                tracing::debug!(what, "core.call: UI gone, result dropped");
                return;
            }
            ctx.request_repaint();
        });
    }

    /// Run `f(state)` on the runtime without waiting for it. Failures are
    /// logged by `what`.
    pub fn fire<F, Fut, T, E>(&self, what: &'static str, f: F)
    where
        F: FnOnce(Arc<AppState>) -> Fut,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        E: std::fmt::Display,
    {
        let fut = f(self.state.clone());
        self.rt.spawn(async move {
            if let Err(e) = fut.await {
                tracing::warn!(what, "core: {e}");
            }
        });
    }

    /// Spawn a raw task (connection writer loops).
    pub fn spawn(&self, fut: impl Future<Output = ()> + Send + 'static) {
        self.rt.spawn(fut);
    }

    /// Block the UI thread on `f(state)`. Only for start-up reads that
    /// decide the first frame (vault status, theme) and take milliseconds.
    pub fn block_on<F, Fut, T>(&self, f: F) -> T
    where
        F: FnOnce(Arc<AppState>) -> Fut,
        Fut: Future<Output = T>,
    {
        self.rt.block_on(f(self.state.clone()))
    }

    pub fn take_jobs(&self) -> Vec<Job> {
        self.jobs_rx.try_iter().collect()
    }
}
