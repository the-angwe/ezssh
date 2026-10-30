//! The spike window: a tab strip, one terminal per tab, and a status line
//! with the numbers the phase 1 gate asks for.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use eframe::egui::{self, Color32, RichText};
use ezssh_term::view::ImeLoopFilter;
use ezssh_term::{
    Faces, FontRegistry, Overlay, Pty, TermAction, TermOptions, TermStyle, Terminal, TerminalView, WindowSize, palette,
};
use zeroize::Zeroizing;

use crate::{Args, TabSpec, local, ssh};

const BENCH_BYTES: usize = 8 * 1024 * 1024;

struct Tab {
    label: String,
    /// How to open it again (Reconnect). `None` for the bench tab.
    spec: Option<TabSpec>,
    term: Terminal,
}

/// A multi-line paste waiting for the user's yes.
struct PendingPaste {
    tab: usize,
    text: String,
    lines: usize,
}

/// The context menu's Font… window.
struct FontPicker {
    tab: usize,
    family: String,
}

/// Discards input; the bench tab has no process behind it.
struct NullPty;

impl Pty for NullPty {
    fn write(&self, _: Cow<'static, [u8]>) {}
    fn resize(&self, _: WindowSize) {}
}

const BENCH_PASSES: usize = 3;

#[derive(Default)]
struct BenchShared {
    /// (start, end) of each pass in nanoseconds since `Bench::epoch`.
    passes: std::sync::Mutex<Vec<(u64, u64)>>,
    done: std::sync::atomic::AtomicBool,
    bytes_per_pass: AtomicU64,
}

struct BenchSample {
    at: u64,
    frame: Duration,
    paint: Duration,
}

struct Bench {
    epoch: Instant,
    shared: Arc<BenchShared>,
    samples: Vec<BenchSample>,
    last_frame: Option<Instant>,
    report: Option<String>,
}

fn pct(sorted: &[Duration], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i].as_secs_f64() * 1000.0
}

impl Bench {
    fn report(&self) -> String {
        let passes = match self.shared.passes.lock() {
            Ok(p) => p.clone(),
            Err(e) => return format!("bench: pass list poisoned: {e}"),
        };
        let mib = self.shared.bytes_per_pass.load(Ordering::Relaxed) as f64 / (1024.0 * 1024.0);
        let mut lines = Vec::new();
        for (i, (start, end)) in passes.iter().enumerate() {
            let secs = (end - start) as f64 / 1e9;
            // The first frame of a pass measures the idle gap before it; skip it.
            let in_pass: Vec<&BenchSample> = self.samples.iter().filter(|s| s.at >= *start && s.at <= *end).skip(1).collect();
            let mut frames: Vec<Duration> = in_pass.iter().map(|s| s.frame).collect();
            let mut paints: Vec<Duration> = in_pass.iter().map(|s| s.paint).collect();
            frames.sort();
            paints.sort();
            lines.push(format!(
                "bench pass {} ({}): {mib:.1} MiB in {:.0} ms = {:.0} MiB/s · {} frames = {:.0} fps · frame p50/p95/max {:.1}/{:.1}/{:.1} ms · paint p50/p95/max {:.2}/{:.2}/{:.2} ms",
                i + 1,
                if i == 0 { "cold" } else { "warm" },
                secs * 1000.0,
                mib / secs,
                in_pass.len(),
                in_pass.len() as f64 / secs,
                pct(&frames, 0.5),
                pct(&frames, 0.95),
                pct(&frames, 1.0),
                pct(&paints, 0.5),
                pct(&paints, 0.95),
                pct(&paints, 1.0),
            ));
        }
        lines.join("\n")
    }
}

pub struct SpikeApp {
    rt: tokio::runtime::Runtime,
    tabs: Vec<Tab>,
    active: usize,
    style: TermStyle,
    fonts: FontRegistry,
    faces: Faces,
    font_size: f32,
    options: TermOptions,
    pending_paste: Option<PendingPaste>,
    font_picker: Option<FontPicker>,
    focus_next: bool,
    renderer: String,
    started: Instant,
    pending_run: Option<String>,
    /// `--find` / `--menu`, applied once the `--run` command had time to print.
    pending_find: Option<String>,
    pending_menu: bool,
    screenshot: Option<(PathBuf, Duration)>,
    screenshot_requested: bool,
    screenshot_done: Arc<std::sync::atomic::AtomicBool>,
    /// Throw-away screenshots taken first. A hidden window renders only the
    /// frames a screenshot forces, and a popup's first frame is egui's
    /// invisible sizing pass.
    screenshot_warmups: u8,
    warmup_done: Option<Arc<std::sync::atomic::AtomicBool>>,
    exit_after: Option<Duration>,
    bench: Option<Bench>,
    // Status-line measurements over one-second windows.
    window_start: Instant,
    window_frames: u32,
    window_bytes: u64,
    fps: f32,
    mib_per_s: f64,
    last_frame: Instant,
    worst_frame_ms: f32,
    window_title: String,
    ime_filter: ImeLoopFilter,
}

fn generated_bench_data() -> Vec<u8> {
    // Coloured, mixed-length lines with some wide and box-drawing text,
    // roughly what `cat` of a log or source tree looks like.
    let mut out = Vec::with_capacity(BENCH_BYTES + 256);
    let words = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa"];
    let mut i: usize = 0;
    while out.len() < BENCH_BYTES {
        let color = 31 + (i % 7);
        let line = match i % 10 {
            0 => format!("\x1b[1;{color}m{i:08}\x1b[0m ── 漢字 テスト ── {}\r\n", words.join(" ")),
            1 => format!("\x1b[38;2;{};{};{}m{i:08} truecolor line\x1b[0m {}\r\n", i % 256, (i * 3) % 256, (i * 7) % 256, words[i % 10].repeat(6)),
            _ => format!("{i:08} \x1b[{color}m{}\x1b[0m {} {}\r\n", words[i % 10], words[(i + 3) % 10].repeat(i % 9), "x".repeat(i % 60)),
        };
        out.extend_from_slice(line.as_bytes());
        i += 1;
    }
    out
}

impl SpikeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, args: Args, password: Option<Zeroizing<String>>) -> Result<Self> {
        let ctx = cc.egui_ctx.clone();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("ezssh-net")
            .build()
            .context("tokio runtime")?;

        let mut fonts = FontRegistry::new();
        let font_value = args.fonts.join(", ");
        let faces = fonts.faces(&ctx, &font_value);
        let style = TermStyle {
            palette: palette::by_id(&args.palette).clone(),
            copy_on_select: args.copy_on_select,
            right_click: args.right_click,
            warn_multiline_paste: args.paste_warning,
            bell: args.bell,
            ligatures: args.ligatures,
            ..TermStyle::default()
        };
        let options = TermOptions {
            scrollback: args.scrollback,
            osc52_write: args.osc52,
            cursor: args.cursor,
            cursor_blink: args.cursor_blink,
        };

        let renderer = match cc.wgpu_render_state.as_ref() {
            Some(rs) => {
                let info = rs.adapter.get_info();
                format!("wgpu {:?} · {} ({:?})", info.backend, info.name, info.device_type)
            }
            None => "glow (OpenGL)".to_string(),
        };
        tracing::info!(%renderer, font = ?faces.loaded, palette = style.palette.id, "spike.window");

        let mut app = SpikeApp {
            rt,
            tabs: Vec::new(),
            active: 0,
            style,
            fonts,
            faces,
            font_size: args.size,
            options,
            pending_paste: None,
            font_picker: None,
            focus_next: true,
            renderer,
            started: Instant::now(),
            pending_run: args.run.clone(),
            pending_find: args.find.clone(),
            pending_menu: args.menu,
            screenshot: args.screenshot.clone().map(|p| (p, Duration::from_secs_f32(args.screenshot_after))),
            screenshot_requested: false,
            screenshot_done: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            screenshot_warmups: if args.find.is_some() || args.menu { 1 } else { 0 },
            warmup_done: None,
            exit_after: args.exit_after.map(Duration::from_secs_f32),
            bench: None,
            window_start: Instant::now(),
            window_frames: 0,
            window_bytes: 0,
            fps: 0.0,
            mib_per_s: 0.0,
            last_frame: Instant::now(),
            worst_frame_ms: 0.0,
            window_title: String::new(),
            ime_filter: ImeLoopFilter::default(),
        };
        for spec in &args.tabs {
            app.open_tab(&ctx, spec, password.clone());
        }
        if let Some(file) = &args.bench {
            app.open_bench(&ctx, file.as_deref())?;
        }
        Ok(app)
    }

    fn spawn(&mut self, ctx: &egui::Context, spec: &TabSpec, password: Option<Zeroizing<String>>) -> Option<(String, Terminal)> {
        let palette = self.style.palette.clone();
        let (label, mut term) = match spec {
            TabSpec::Local(shell) => match local::spawn(ctx, shell.as_deref(), self.options.clone(), palette) {
                Ok(term) => (shell.clone().unwrap_or_else(|| "local".into()), term),
                Err(e) => {
                    tracing::error!("spike: local shell failed: {e:#}");
                    return None;
                }
            },
            TabSpec::Ssh(target) => {
                let term = ssh::spawn(self.rt.handle(), ctx, target.clone(), password, self.options.clone(), palette);
                (target.label(), term)
            }
        };
        term.set_faces(self.faces.clone());
        term.set_font_size(self.font_size);
        Some((label, term))
    }

    fn open_tab(&mut self, ctx: &egui::Context, spec: &TabSpec, password: Option<Zeroizing<String>>) {
        if let Some((label, term)) = self.spawn(ctx, spec, password) {
            self.tabs.push(Tab { label, spec: Some(spec.clone()), term });
        }
        self.active = self.tabs.len().saturating_sub(1);
        self.focus_next = true;
    }

    fn reconnect(&mut self, ctx: &egui::Context, i: usize) {
        let Some(spec) = self.tabs.get(i).and_then(|t| t.spec.clone()) else {
            tracing::warn!(tab = i, "spike: this tab cannot reconnect");
            return;
        };
        tracing::info!(tab = i, ?spec, "spike: reconnect");
        if let Some((_, mut term)) = self.spawn(ctx, &spec, None) {
            let old = &self.tabs[i].term;
            term.set_faces(old.faces().clone());
            term.set_font_size(old.font_size());
            self.tabs[i].term = term;
            self.focus_next = true;
        }
    }

    fn handle_actions(&mut self, ctx: &egui::Context, tab: usize, actions: Vec<TermAction>) {
        for action in actions {
            tracing::debug!(tab, ?action, "spike.action");
            match action {
                TermAction::Reconnect => self.reconnect(ctx, tab),
                TermAction::ConfirmPaste { text, lines } => self.pending_paste = Some(PendingPaste { tab, text, lines }),
                TermAction::OpenUrl(url) => {
                    if let Err(e) = open::that_detached(&url) {
                        tracing::error!(%url, "spike: opening link failed: {e}");
                    }
                }
                TermAction::Bell => tracing::info!(tab, "spike: bell (sound mode; the app plays it)"),
                TermAction::OpenFontPicker => {
                    let family = self.tabs[tab].term.faces().loaded.clone().unwrap_or_default();
                    self.font_picker = Some(FontPicker { tab, family });
                }
            }
        }
    }

    fn show_modals(&mut self, ctx: &egui::Context) {
        if let Some(p) = &self.pending_paste {
            let mut answer = None;
            egui::Modal::new(egui::Id::unique("paste-confirm")).show(ctx, |ui| {
                ui.heading("Paste multiple lines?");
                ui.label(format!("The clipboard holds {} lines. Each one will run as you paste it.", p.lines));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Paste").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        answer = Some(false);
                    }
                });
            });
            if let Some(yes) = answer {
                if let Some(p) = self.pending_paste.take()
                    && yes
                    && let Some(t) = self.tabs.get_mut(p.tab)
                {
                    t.term.paste(&p.text);
                }
                self.focus_next = true;
            }
        }
        if let Some(picker) = &mut self.font_picker {
            let mut open = true;
            let mut apply = false;
            let tab = picker.tab;
            egui::Window::new("Font").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
                if let Some(t) = self.tabs.get_mut(tab) {
                    ui.horizontal(|ui| {
                        ui.label("Size");
                        let mut size = t.term.font_size();
                        if ui.add(egui::DragValue::new(&mut size).range(8.0..=48.0).speed(0.2)).changed() {
                            t.term.set_font_size(size.round());
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Family");
                        apply = ui.text_edit_singleline(&mut picker.family).lost_focus();
                        apply |= ui.button("Apply").clicked();
                    });
                    ui.label(RichText::new(format!("loaded: {}", t.term.faces().loaded.as_deref().unwrap_or("egui mono"))).small().weak());
                }
            });
            if apply {
                let faces = self.fonts.faces(ctx, &picker.family);
                if let Some(t) = self.tabs.get_mut(tab) {
                    t.term.set_faces(faces);
                }
            }
            if !open {
                self.font_picker = None;
                self.focus_next = true;
            }
        }
    }

    fn open_bench(&mut self, ctx: &egui::Context, file: Option<&Path>) -> Result<()> {
        let data = match file {
            Some(p) => std::fs::read(p).with_context(|| format!("reading bench file {}", p.display()))?,
            None => generated_bench_data(),
        };
        let (mut term, feeder) = Terminal::new(ctx, Arc::new(NullPty), self.options.clone(), self.style.palette.clone());
        term.set_faces(self.faces.clone());
        term.set_font_size(self.font_size);
        let shared = Arc::new(BenchShared::default());
        shared.bytes_per_pass.store(data.len() as u64, Ordering::Relaxed);
        let epoch = Instant::now();
        let s = shared.clone();
        std::thread::Builder::new()
            .name("bench-feeder".into())
            .spawn(move || {
                // Let the window open and size the grid first.
                std::thread::sleep(Duration::from_millis(1500));
                for pass in 0..BENCH_PASSES {
                    let start = epoch.elapsed().as_nanos() as u64;
                    for chunk in data.chunks(64 * 1024) {
                        feeder.feed(chunk);
                    }
                    let end = epoch.elapsed().as_nanos() as u64;
                    tracing::debug!(pass, ms = (end - start) / 1_000_000, "bench: pass fed");
                    match s.passes.lock() {
                        Ok(mut p) => p.push((start, end)),
                        Err(e) => tracing::error!("bench: pass list poisoned: {e}"),
                    }
                    // Let the last frames of the pass land before the next one.
                    std::thread::sleep(Duration::from_millis(300));
                }
                s.done.store(true, Ordering::SeqCst);
                feeder.exited("bench done");
            })
            .context("bench thread")?;
        self.bench = Some(Bench { epoch, shared, samples: Vec::new(), last_frame: None, report: None });
        self.tabs.push(Tab { label: "bench".into(), spec: None, term });
        self.active = self.tabs.len() - 1;
        self.focus_next = true;
        Ok(())
    }

    fn measure(&mut self) {
        let now = Instant::now();
        let dt = now - self.last_frame;
        self.last_frame = now;
        self.window_frames += 1;
        let frame_ms = dt.as_secs_f32() * 1000.0;
        // Ignore idle gaps: egui only repaints on demand.
        if frame_ms < 250.0 {
            self.worst_frame_ms = self.worst_frame_ms.max(frame_ms);
        }
        let bytes: u64 = self.tabs.iter().map(|t| t.term.stats().bytes.load(Ordering::Relaxed)).sum();
        let elapsed = now - self.window_start;
        if elapsed >= Duration::from_secs(1) {
            let secs = elapsed.as_secs_f64();
            self.fps = (self.window_frames as f64 / secs) as f32;
            self.mib_per_s = (bytes - self.window_bytes) as f64 / secs / (1024.0 * 1024.0);
            if bytes != self.window_bytes {
                tracing::info!(
                    fps = format!("{:.0}", self.fps),
                    worst_frame_ms = format!("{:.1}", self.worst_frame_ms),
                    mib_per_s = format!("{:.1}", self.mib_per_s),
                    "spike.stats"
                );
            } else {
                tracing::debug!(fps = format!("{:.0}", self.fps), "spike.stats idle");
            }
            self.window_bytes = bytes;
            self.window_frames = 0;
            self.window_start = now;
            self.worst_frame_ms = 0.0;
        }

        // Bench: frame and paint times while the feeder runs.
        if let Some(b) = &mut self.bench
            && b.report.is_none()
        {
            let paint = self.tabs.iter().find(|t| t.label == "bench").map(|t| t.term.view_state().last_paint()).unwrap_or_default();
            if let Some(last) = b.last_frame {
                b.samples.push(BenchSample { at: b.epoch.elapsed().as_nanos() as u64, frame: now - last, paint });
            }
            b.last_frame = Some(now);
            if b.shared.done.load(Ordering::SeqCst) {
                let report = b.report();
                tracing::info!("{report}");
                println!("{report}");
                b.report = Some(report.lines().last().unwrap_or_default().to_string());
            }
        }
    }

    fn handle_automation(&mut self, ctx: &egui::Context) {
        let since = self.started.elapsed();
        tracing::trace!(since_ms = since.as_millis() as u64, "spike.frame");
        const RUN_DELAY: Duration = Duration::from_millis(600);
        if let Some(cmd) = &self.pending_run {
            if since < RUN_DELAY {
                // Nothing else may repaint while the shell starts.
                ctx.request_repaint_after(RUN_DELAY - since);
            } else if let Some(tab) = self.tabs.first() {
                tracing::info!(%cmd, "spike: --run");
                tab.term.write(format!("{cmd}\r").into_bytes());
                self.pending_run = None;
            }
        }
        const POPUP_DELAY: Duration = Duration::from_millis(1800);
        if (self.pending_find.is_some() || self.pending_menu) && self.pending_run.is_none() {
            if since < POPUP_DELAY {
                ctx.request_repaint_after(POPUP_DELAY - since);
            } else if let Some(tab) = self.tabs.get_mut(self.active) {
                if let Some(q) = self.pending_find.take() {
                    tracing::info!(query = %q, "spike: --find");
                    tab.term.find(&q);
                }
                if std::mem::take(&mut self.pending_menu) {
                    tracing::info!("spike: --menu");
                    tab.term.open_menu(egui::pos2(300.0, 200.0));
                }
                ctx.request_repaint();
            }
        }
        if let Some(flag) = &self.warmup_done {
            if flag.load(Ordering::SeqCst) {
                tracing::debug!("spike: warm-up screenshot done");
                self.warmup_done = None;
                self.screenshot_warmups -= 1;
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        if let Some((_, after)) = &self.screenshot
            && !self.screenshot_requested
            && self.warmup_done.is_none()
        {
            if since >= *after && self.screenshot_warmups > 0 {
                tracing::debug!("spike: warm-up screenshot requested");
                let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let f = flag.clone();
                let wake = ctx.clone();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |_| {
                    f.store(true, Ordering::SeqCst);
                    wake.request_repaint();
                })));
                self.warmup_done = Some(flag);
            } else if since >= *after {
                if let Some((path, _)) = self.screenshot.take() {
                    tracing::debug!(path = %path.display(), "spike: screenshot requested");
                    let done = self.screenshot_done.clone();
                    let wake = ctx.clone();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |image| {
                        match save_png(&path, &image) {
                            Ok(()) => tracing::info!(path = %path.display(), "spike: screenshot saved"),
                            Err(e) => tracing::error!(path = %path.display(), "spike: screenshot failed: {e:#}"),
                        }
                        done.store(true, Ordering::SeqCst);
                        wake.request_repaint();
                    })));
                }
                self.screenshot_requested = true;
            } else {
                ctx.request_repaint_after(*after - since);
            }
        }
        if let Some(after) = self.exit_after {
            // A pending screenshot holds the exit, for at most 45 seconds: a
            // hidden Wayland window gets a frame only every ~10 s.
            let shot_pending = (self.screenshot.is_some() || self.screenshot_requested)
                && !self.screenshot_done.load(Ordering::SeqCst);
            if shot_pending && since < after + Duration::from_secs(45) {
                ctx.request_repaint_after(Duration::from_millis(100));
            } else if since >= after {
                if shot_pending {
                    tracing::error!("spike: screenshot never completed");
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ctx.request_repaint_after(after - since);
            }
        }
    }
}

fn save_png(path: &Path, image: &egui::ColorImage) -> Result<()> {
    let [w, h] = image.size;
    let file = std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().context("png header")?;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect();
    writer.write_image_data(&rgba).context("png data")?;
    Ok(())
}

impl eframe::App for SpikeApp {
    /// Runs even while the window is hidden (eframe skips `ui` then), so
    /// timers live here.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_automation(ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.ime_filter.filter(raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if tracing::enabled!(tracing::Level::TRACE) {
            let events: Vec<String> = ctx.input(|i| i.raw.events.iter().map(|e| format!("{e:?}").chars().take(80).collect()).collect());
            tracing::trace!(causes = ?ctx.repaint_causes(), ?events, "spike.repaint");
        }
        self.measure();

        egui::Panel::top("tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                let mut clicked = None;
                for (i, tab) in self.tabs.iter().enumerate() {
                    let title = tab.term.title().map(str::to_string).unwrap_or_else(|| tab.label.clone());
                    let exited = tab.term.exit_reason().is_some();
                    let text = if exited { RichText::new(title).color(Color32::GRAY) } else { RichText::new(title) };
                    if ui.selectable_label(i == self.active, text).clicked() {
                        clicked = Some(i);
                    }
                }
                if let Some(i) = clicked {
                    self.active = i;
                    self.focus_next = true;
                }
                if ui.button("+ local").clicked() {
                    self.open_tab(&ctx, &TabSpec::Local(None), None);
                }
                ui.separator();
                let current = self.style.palette.id;
                let mut chosen = None;
                egui::ComboBox::from_id_salt("palette").selected_text(self.style.palette.label).show_ui(ui, |ui| {
                    for p in palette::PALETTES {
                        if ui.selectable_label(p.id == current, p.label).clicked() {
                            chosen = Some(p);
                        }
                    }
                });
                if let Some(p) = chosen {
                    tracing::info!(palette = p.id, "spike: palette");
                    self.style.palette = p.clone();
                    for t in &self.tabs {
                        t.term.set_palette(p.clone());
                    }
                }
            });
        });

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let (cols, rows) = self.tabs.get(self.active).map(|t| t.term.grid_size()).unwrap_or((0, 0));
                let paint_ms = self.tabs.get(self.active).map(|t| t.term.view_state().last_paint().as_secs_f32() * 1000.0).unwrap_or(0.0);
                ui.label(
                    RichText::new(format!(
                        "{:.0} fps · worst {:.1} ms · paint {:.2} ms · {:.1} MiB/s · {cols}×{rows} · {} {:.0}pt · {}",
                        self.fps,
                        self.worst_frame_ms,
                        paint_ms,
                        self.mib_per_s,
                        self.tabs.get(self.active).and_then(|t| t.term.faces().loaded.clone()).unwrap_or_else(|| "egui mono".into()),
                        self.tabs.get(self.active).map_or(self.font_size, |t| t.term.font_size()),
                        self.renderer,
                    ))
                    .monospace()
                    .small(),
                );
                if let Some(report) = self.bench.as_ref().and_then(|b| b.report.as_ref()) {
                    ui.separator();
                    ui.label(RichText::new(report).small().color(Color32::LIGHT_GREEN));
                }
            });
        });

        let bg = {
            let c = self.style.palette.background;
            Color32::from_rgb(c.r, c.g, c.b)
        };
        let mut actions = Vec::new();
        let modal_open = self.pending_paste.is_some() || self.font_picker.is_some();
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(bg)).show(ui, |ui| {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                let focus = !modal_open && std::mem::take(&mut self.focus_next);
                // The spike has no connection state machine: an ended byte
                // source shows the Disconnected card (the bench tab keeps
                // its last screen).
                let overlay = tab.term.exit_reason().filter(|_| tab.spec.is_some()).map(|reason| Overlay::Closed {
                    reason: Some(reason),
                    summary: tab.label.clone(),
                });
                let out = TerminalView::new(&mut tab.term, &self.style).focus(focus).overlay(overlay.as_ref()).show(ui);
                actions = out.actions;
            } else {
                ui.centered_and_justified(|ui| ui.label("no tabs"));
            }
        });
        let active = self.active;
        self.handle_actions(&ctx, active, actions);
        self.show_modals(&ctx);

        let title = self
            .tabs
            .get(self.active)
            .map(|t| t.term.title().map(str::to_string).unwrap_or_else(|| t.label.clone()))
            .unwrap_or_default();
        if title != self.window_title {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(format!("{title} — ezSsh spike")));
            self.window_title = title;
        }
    }
}
