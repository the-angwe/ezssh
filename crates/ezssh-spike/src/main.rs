//! ezSsh phase 1 spike: the native terminal widget in a bare eframe window.
//!
//! Not the product. It exists to answer the go / no-go questions of the
//! native rework (docs/design/native-rework-plan.md §5, phase 1).

mod app;
mod local;
mod ssh;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, bail};
use eframe::egui;
use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew};
use eframe::wgpu;
use zeroize::Zeroizing;

const HELP: &str = "\
ezssh-spike — ezSsh native terminal spike

USAGE:
    ezssh-spike [OPTIONS]

TABS (repeatable; default is one local shell):
    --local                 Open a local shell tab (default shell)
    --shell PATH            Open a local tab running PATH
    --ssh USER@HOST[:PORT]  Open an SSH tab

SSH:
    --key PATH              Private key file (default: agent, then ~/.ssh/id_*)
    --password              Ask for a password / key passphrase before the window opens
    --trust-new-host        Accept and remember an unknown host key

LOOK:
    --font NAME             Terminal font family (repeatable, first installed wins)
    --size PT               Font size in points (8–48, default 14)
    --scrollback LINES      Scrollback lines (default 10000)

RENDERING:
    --renderer wgpu|warp|glow
                            wgpu picks the best GPU and falls back to a software
                            adapter (WARP on Windows); warp forces the software
                            adapter; glow uses OpenGL

TESTING:
    --run CMD               Type CMD + Enter into the first tab once it starts
    --bench [FILE]          Open a bench tab that pours FILE (default: 8 MiB of
                            generated coloured text) through the terminal and
                            reports throughput and frame rate
    --screenshot PATH       Save a PNG of the window (needs --screenshot-after)
    --screenshot-after SECS Seconds to wait before the screenshot (default 2)
    --exit-after SECS       Close the window after SECS
    -h, --help              Show this help

Logging: RUST_LOG=ezssh_term=debug,ezssh_spike=debug
";

#[derive(Clone, Debug)]
pub enum TabSpec {
    Local(Option<String>),
    Ssh(ssh::Target),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererChoice {
    Wgpu,
    Warp,
    Glow,
}

#[derive(Debug)]
pub struct Args {
    pub tabs: Vec<TabSpec>,
    pub key: Option<PathBuf>,
    pub ask_password: bool,
    pub trust_new_host: bool,
    pub fonts: Vec<String>,
    pub size: f32,
    pub scrollback: usize,
    pub renderer: RendererChoice,
    pub run: Option<String>,
    pub bench: Option<Option<PathBuf>>,
    pub screenshot: Option<PathBuf>,
    pub screenshot_after: f32,
    pub exit_after: Option<f32>,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        tabs: Vec::new(),
        key: None,
        ask_password: false,
        trust_new_host: false,
        fonts: Vec::new(),
        size: 14.0,
        scrollback: 10_000,
        renderer: RendererChoice::Wgpu,
        run: None,
        bench: None,
        screenshot: None,
        screenshot_after: 2.0,
        exit_after: None,
    };
    let mut it = std::env::args().skip(1).peekable();
    let value = |it: &mut std::iter::Peekable<std::iter::Skip<std::env::Args>>, flag: &str| {
        it.next().ok_or_else(|| anyhow!("{flag} needs a value"))
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "--local" => args.tabs.push(TabSpec::Local(None)),
            "--shell" => args.tabs.push(TabSpec::Local(Some(value(&mut it, &a)?))),
            "--ssh" => args.tabs.push(TabSpec::Ssh(ssh::Target::parse(&value(&mut it, &a)?)?)),
            "--key" => args.key = Some(PathBuf::from(value(&mut it, &a)?)),
            "--password" => args.ask_password = true,
            "--trust-new-host" => args.trust_new_host = true,
            "--font" => args.fonts.push(value(&mut it, &a)?),
            "--size" => {
                let v: f32 = value(&mut it, &a)?.parse().context("--size")?;
                args.size = v.clamp(ezssh_term::view::MIN_FONT_SIZE, ezssh_term::view::MAX_FONT_SIZE);
            }
            "--scrollback" => args.scrollback = value(&mut it, &a)?.parse().context("--scrollback")?,
            "--renderer" => {
                args.renderer = match value(&mut it, &a)?.as_str() {
                    "wgpu" => RendererChoice::Wgpu,
                    "warp" => RendererChoice::Warp,
                    "glow" => RendererChoice::Glow,
                    other => bail!("unknown renderer {other:?} (wgpu, warp, glow)"),
                }
            }
            "--run" => args.run = Some(value(&mut it, &a)?),
            "--bench" => {
                let file = match it.peek() {
                    Some(v) if !v.starts_with("--") => Some(PathBuf::from(it.next().unwrap_or_default())),
                    _ => None,
                };
                args.bench = Some(file);
            }
            "--screenshot" => args.screenshot = Some(PathBuf::from(value(&mut it, &a)?)),
            "--screenshot-after" => args.screenshot_after = value(&mut it, &a)?.parse().context("--screenshot-after")?,
            "--exit-after" => args.exit_after = Some(value(&mut it, &a)?.parse().context("--exit-after")?),
            other => bail!("unknown argument {other:?}; see --help"),
        }
    }
    for t in &mut args.tabs {
        if let TabSpec::Ssh(target) = t {
            target.key = args.key.clone();
            target.trust_new_host = args.trust_new_host;
        }
    }
    if args.tabs.is_empty() && args.bench.is_none() {
        args.tabs.push(TabSpec::Local(None));
    }
    Ok(args)
}

/// Adapter choice: best hardware adapter that can present to the window,
/// else a software one (WARP on Windows, llvmpipe / lavapipe on Linux).
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
        tracing::info!(name = %info.name, backend = ?info.backend, device_type = ?info.device_type, driver = %info.driver, presents, "wgpu.adapter");
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

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn")),
        )
        .init();
    let args = parse_args()?;
    tracing::info!(?args, "spike.start");

    let password = if args.ask_password {
        let pw = rpassword::prompt_password("SSH password / key passphrase: ").context("reading password")?;
        Some(Zeroizing::new(pw))
    } else {
        None
    };

    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ezSsh spike")
            .with_app_id("ezssh-spike")
            .with_inner_size([1100.0, 700.0]),
        // Ctrl+C / Ctrl+V / Ctrl+X must reach the shell as keys.
        clipboard_shortcuts: false,
        ..Default::default()
    };
    match args.renderer {
        RendererChoice::Glow => options.renderer = eframe::Renderer::Glow,
        RendererChoice::Wgpu | RendererChoice::Warp => {
            options.renderer = eframe::Renderer::Wgpu;
            let software_only = args.renderer == RendererChoice::Warp;
            let mut setup = WgpuSetupCreateNew::without_display_handle();
            setup.native_adapter_selector = Some(Arc::new(move |adapters, surface| select_adapter(adapters, surface, software_only)));
            options.wgpu_options.wgpu_setup = WgpuSetup::CreateNew(setup);
        }
    }

    eframe::run_native("ezssh-spike", options, Box::new(move |cc| Ok(Box::new(app::SpikeApp::new(cc, args, password)?))))
        .map_err(|e| anyhow!("eframe: {e}"))
}
