//! Toasts: `ui/lib/toast.ts` (four kinds, 4 / 6 / 8 / 4 s) and
//! `toast-region.tsx` (bottom-right stack, newest at the bottom).

use std::time::{Duration, Instant};

use egui::{Align2, Color32, CornerRadius, Frame, Id, Margin, Order, RichText, Stroke, vec2};

use crate::icons::Icon;
use crate::style::{alpha, font, font_semibold, size, tokens};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Success,
    Warning,
    Danger,
    Info,
}

impl Kind {
    fn ttl(self) -> Duration {
        Duration::from_millis(match self {
            Kind::Success | Kind::Info => 4000,
            Kind::Warning => 6000,
            Kind::Danger => 8000,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Toast {
    id: u64,
    pub kind: Kind,
    pub title: String,
    pub body: Option<String>,
    until: Instant,
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next_id: u64,
}

impl Toasts {
    pub fn push(&mut self, kind: Kind, title: impl Into<String>, body: Option<String>) {
        let title = title.into();
        match kind {
            Kind::Danger => tracing::warn!(%title, ?body, "toast"),
            _ => tracing::info!(?kind, %title, ?body, "toast"),
        }
        self.next_id += 1;
        self.items.push(Toast { id: self.next_id, kind, title, body, until: Instant::now() + kind.ttl() });
    }

    pub fn success(&mut self, title: impl Into<String>, body: impl Into<Option<String>>) {
        self.push(Kind::Success, title, body.into());
    }
    pub fn warning(&mut self, title: impl Into<String>, body: impl Into<Option<String>>) {
        self.push(Kind::Warning, title, body.into());
    }
    pub fn danger(&mut self, title: impl Into<String>, body: impl Into<Option<String>>) {
        self.push(Kind::Danger, title, body.into());
    }
    pub fn info(&mut self, title: impl Into<String>, body: impl Into<Option<String>>) {
        self.push(Kind::Info, title, body.into());
    }

    #[cfg(test)]
    pub fn items(&self) -> &[Toast] {
        &self.items
    }

    /// Drop expired toasts; returns when the next one expires. Called from
    /// `App::logic` so toasts also expire while the window is hidden.
    pub fn expire(&mut self, now: Instant) -> Option<Duration> {
        self.items.retain(|t| t.until > now);
        self.items.iter().map(|t| t.until - now).min()
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        if self.items.is_empty() {
            return;
        }
        let t = tokens(ctx);
        let mut dismiss = None;
        egui::Area::new(Id::unique("toasts"))
            .order(Order::Tooltip)
            .anchor(Align2::RIGHT_BOTTOM, vec2(-12.0, -32.0))
            .show(ctx, |ui| {
                let width = 320.0_f32.min(ctx.content_rect().width() - 24.0);
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 8.0;
                for toast in &self.items {
                    let (icon, color) = match toast.kind {
                        Kind::Success => (Icon::CheckCircle, t.success),
                        Kind::Warning => (Icon::AlertTriangle, t.warning),
                        Kind::Danger => (Icon::AlertCircle, t.danger),
                        Kind::Info => (Icon::Info, t.accent),
                    };
                    let card = Frame::new()
                        .fill(t.surface)
                        .stroke(Stroke::new(1.0, t.border))
                        .corner_radius(CornerRadius::same(2))
                        .inner_margin(Margin { left: 12, right: 10, top: 10, bottom: 10 })
                        .shadow(egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(90) })
                        .show(ui, |ui| {
                            ui.set_width(width - 22.0);
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                icon.show(ui, 14.0, color);
                                let text_w = ui.available_width() - 28.0;
                                ui.vertical(|ui| {
                                    ui.set_width(text_w);
                                    ui.add(
                                        egui::Label::new(RichText::new(&toast.title).font(font_semibold(size::XS)).color(t.fg))
                                            .truncate(),
                                    );
                                    if let Some(body) = &toast.body {
                                        ui.add(egui::Label::new(RichText::new(body).font(font(size::XS)).color(t.muted)).wrap());
                                    }
                                });
                                if crate::widgets::icon_button(ui, Icon::X, 20.0, 12.0, false, "Dismiss").clicked() {
                                    dismiss = Some(toast.id);
                                }
                            });
                        });
                    // Kind bar on the left edge.
                    let r = card.response.rect;
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(r.min, vec2(2.0, r.height())),
                        CornerRadius { nw: 2, sw: 2, ne: 0, se: 0 },
                        alpha(color, 1.0),
                    );
                }
            });
        if let Some(id) = dismiss {
            self.items.retain(|t| t.id != id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttls_and_expiry() {
        let mut t = Toasts::default();
        t.success("a", None);
        t.danger("b", Some("why".into()));
        let now = Instant::now();
        let next = t.expire(now).unwrap();
        assert!(next <= Duration::from_millis(4000) && next > Duration::from_millis(3900));
        assert_eq!(t.expire(now + Duration::from_millis(4100)).map(|d| d.as_millis() / 1000), Some(3));
        assert_eq!(t.items().len(), 1);
        assert_eq!(t.items()[0].title, "b");
        assert_eq!(t.expire(now + Duration::from_secs(9)), None);
        assert!(t.items().is_empty());
    }
}
