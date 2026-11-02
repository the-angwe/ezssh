//! `ezssh`: the native ezSsh window.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, bail};
use eframe::egui;
use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew};
use eframe::wgpu;

const HELP: &str = "\
ezssh — ezSsh, native build

USAGE:
    ezssh [OPTIONS]

OPTIONS:
    --db PATH               Database file (default: the ezTerm data path,
                            shared with the Tauri build)
    --renderer wgpu|warp|glow
                            wgpu picks the best GPU and falls back to a software
                            adapter (WARP on Windows); warp forces software;
                            glow uses OpenGL
    -h, --help              Show this help

Logging: RUST_LOG=ezssh_app=debug,ezssh_core=debug,ezssh_term=debug
Updates: EZSSH_UPDATE_MANIFEST=URL reads that latest.json instead of the
         GitHub release one (installers must still carry a valid signature)
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Renderer {
    Wgpu,
    Warp,
    Glow,
}

struct Args {
    db: Option<PathBuf>,
    renderer: Renderer,
}

fn parse_args() -> Result<Args> {
    let mut args = Args { db: None, renderer: Renderer::Wgpu };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "--db" => args.db = Some(PathBuf::from(it.next().ok_or_else(|| anyhow!("--db needs a path"))?)),
            "--renderer" => {
                args.renderer = match it.next().as_deref() {
                    Some("wgpu") => Renderer::Wgpu,
                    Some("warp") => Renderer::Warp,
                    Some("glow") => Renderer::Glow,
                    other => bail!("unknown renderer {other:?} (wgpu, warp, glow)"),
                }
            }
            other => bail!("unknown argument {other:?}; see --help"),
        }
    }
    Ok(args)
}

/// Best hardware adapter that can present to the window, else a software
/// one (WARP on Windows, llvmpipe / lavapipe on Linux): VMs and RDP
/// sessions often have no GPU.
fn select_adapter(adapters: &[wgpu::Adapter], surface: Option<&wgpu::Surface<'_>>, software_only: bool) -> Result<wgpu::Adapter, String> {
    let rank = |t: wgpu::DeviceType| match t {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::Cpu => 4,
    };
    let mut usable: Vec<&wgpu::Adapter> = Vec::new();
    for a in adapters {
        let info = a.get_info();
        let presents = surface.is_none_or(|s| a.is_surface_supported(s));
        tracing::info!(name = %info.name, backend = ?info.backend, device_type = ?info.device_type, presents, "wgpu.adapter");
        if presents && (!software_only || info.device_type == wgpu::DeviceType::Cpu) {
            usable.push(a);
        }
    }
    usable.sort_by_key(|a| rank(a.get_info().device_type));
    match usable.first() {
        Some(a) => {
            let info = a.get_info();
            tracing::info!(name = %info.name, backend = ?info.backend, device_type = ?info.device_type, "wgpu.adapter: selected");
            Ok((*a).clone())
        }
        None => Err(format!(
            "no {}adapter can present to this window ({} enumerated); try --renderer glow",
            if software_only { "software " } else { "" },
            adapters.len()
        )),
    }
}

/// Shown instead of the app when start-up fails (database unreadable).
struct StartupError(String);

impl eframe::App for StartupError {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("ezSsh could not start");
            ui.add_space(8.0);
            ui.label(&self.0);
            ui.add_space(12.0);
            if ui.button("Quit").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn")),
        )
        .init();
    let args = parse_args()?;
    let db_path = match args.db {
        Some(p) => p,
        None => ezssh_core::state::default_db_path().context("locating the data directory")?,
    };
    tracing::info!(db = %db_path.display(), renderer = ?args.renderer, version = env!("CARGO_PKG_VERSION"), "ezssh.start");

    let mut viewport = egui::ViewportBuilder::default();
    match eframe::icon_data::from_png_bytes(include_bytes!("../packaging/icons/256x256.png")) {
        Ok(icon) => viewport = viewport.with_icon(Arc::new(icon)),
        Err(e) => tracing::warn!("ezssh: window icon unreadable: {e}"),
    }
    let mut options = eframe::NativeOptions {
        viewport: viewport
            .with_title("ezSsh")
            .with_app_id("ezssh")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        // Ctrl+C / Ctrl+V / Ctrl+X must reach the shell as keys.
        clipboard_shortcuts: false,
        ..Default::default()
    };
    match args.renderer {
        Renderer::Glow => options.renderer = eframe::Renderer::Glow,
        Renderer::Wgpu | Renderer::Warp => {
            options.renderer = eframe::Renderer::Wgpu;
            let software_only = args.renderer == Renderer::Warp;
            let mut setup = WgpuSetupCreateNew::without_display_handle();
            setup.native_adapter_selector = Some(Arc::new(move |adapters, surface| select_adapter(adapters, surface, software_only)));
            options.wgpu_options.wgpu_setup = WgpuSetup::CreateNew(setup);
        }
    }

    let manifest = std::env::var("EZSSH_UPDATE_MANIFEST").ok().filter(|v| !v.is_empty());
    if let Some(m) = &manifest {
        tracing::warn!(manifest = %m, "ezssh: update manifest overridden (signatures are still checked)");
    }
    let updater = ezssh_core::updater::Config::new(ezssh_core::updater::detect(), manifest);
    let app_options = ezssh_app::Options { db_path, updater: Some(updater) };
    eframe::run_native(
        "ezssh",
        options,
        Box::new(move |cc| {
            Ok(match ezssh_app::App::new(&cc.egui_ctx, app_options) {
                Ok(app) => Box::new(app) as Box<dyn eframe::App>,
                Err(e) => {
                    tracing::error!("ezssh: start-up failed: {e:#}");
                    Box::new(StartupError(format!("{e:#}")))
                }
            })
        }),
    )
    .map_err(|e| anyhow!("eframe: {e}"))
}
