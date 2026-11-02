//! Lucide icons (the set `lucide-react` draws in the webview build),
//! vendored as white-stroke SVGs by `tools/gen_icons.mjs` and tinted per
//! use. egui_extras' SVG loader rasterises each icon once per pixel size.

use egui::{Color32, Image, ImageSource, Rect, Response, Sense, Ui, Vec2};

macro_rules! icons {
    ($($variant:ident => $file:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Icon { $($variant),* }

        impl Icon {
            pub fn source(self) -> ImageSource<'static> {
                match self {
                    $(Icon::$variant => egui::include_image!(concat!("../assets/icons/", $file, ".svg"))),*
                }
            }

            /// Every icon, for the asset test.
            #[cfg(test)]
            pub const ALL: &[Icon] = &[$(Icon::$variant),*];
        }
    };
}

icons! {
    AlertCircle => "circle-alert",
    AlertTriangle => "triangle-alert",
    ArrowLeftRight => "arrow-left-right",
    Check => "check",
    CheckCircle => "circle-check",
    ChevronDown => "chevron-down",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    Cloud => "cloud",
    CloudOff => "cloud-off",
    CloudUpload => "cloud-upload",
    Cog => "cog",
    Columns2 => "columns-2",
    Copy => "copy",
    Cpu => "cpu",
    Download => "download",
    Eye => "eye",
    EyeOff => "eye-off",
    File => "file",
    FileInput => "file-input",
    FileSymlink => "file-symlink",
    Folder => "folder",
    FolderOpen => "folder-open",
    FolderPlus => "folder-plus",
    FolderTree => "folder-tree",
    Globe2 => "earth",
    Grid3x3 => "grid-3x3",
    Inbox => "inbox",
    Info => "info",
    Key => "key",
    KeyRound => "key-round",
    KeySquare => "key-square",
    Layers => "layers",
    LayoutGrid => "layout-grid",
    Loader2 => "loader-circle",
    Lock => "lock",
    Maximize2 => "maximize-2",
    Minimize2 => "minimize-2",
    Minus => "minus",
    Monitor => "monitor",
    MonitorDot => "monitor-dot",
    MonitorOff => "monitor-off",
    Moon => "moon",
    Network => "network",
    PackageCheck => "package-check",
    PackageOpen => "package-open",
    Palette => "palette",
    PanelLeftClose => "panel-left-close",
    PanelLeftOpen => "panel-left-open",
    Pencil => "pencil",
    Play => "play",
    PlugZap => "plug-zap",
    Plus => "plus",
    RefreshCcw => "refresh-ccw",
    RefreshCw => "refresh-cw",
    Rows2 => "rows-2",
    Server => "server",
    Settings => "settings",
    Settings2 => "settings-2",
    Shield => "shield",
    ShieldAlert => "shield-alert",
    ShieldCheck => "shield-check",
    ShieldX => "shield-x",
    Slash => "slash",
    Sliders => "sliders-vertical",
    Sparkles => "sparkles",
    Square => "square",
    SquareStack => "square-stack",
    SquareTerminal => "square-terminal",
    Sun => "sun",
    Terminal => "terminal",
    Trash => "trash",
    Type => "type",
    Upload => "upload",
    User => "user",
    X => "x",
}

impl Icon {
    pub fn image(self, size: f32, color: Color32) -> Image<'static> {
        Image::new(self.source()).fit_to_exact_size(Vec2::splat(size)).tint(color)
    }

    /// Paint centred in `rect` (no interaction).
    pub fn paint(self, ui: &Ui, rect: Rect, size: f32, color: Color32) {
        let r = Rect::from_center_size(rect.center(), Vec2::splat(size));
        self.image(size, color).paint_at(ui, r);
    }

    /// Allocate `size` and paint the icon there.
    pub fn show(self, ui: &mut Ui, size: f32, color: Color32) -> Response {
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        self.paint(ui, rect, size, color);
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_an_svg() {
        for icon in Icon::ALL {
            let ImageSource::Bytes { uri, bytes } = icon.source() else {
                panic!("{icon:?}: not embedded bytes");
            };
            assert!(uri.ends_with(".svg"), "{icon:?}: {uri} must end in .svg for the SVG loader");
            let text = std::str::from_utf8(&bytes).expect("utf-8");
            assert!(text.starts_with("<svg") && text.contains("stroke=\"#ffffff\""), "{icon:?}");
        }
    }
}
