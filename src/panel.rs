use crate::{app::PanelResize, browser::Browser, theme};
use gpui::{prelude::*, *};

const PANEL_MIN: f32 = 480.;
const TERMINAL_MIN: f32 = 240.;
const OVERLAY_MARGIN: f32 = 4.;
const DIVIDER_WIDTH: f32 = 1.;
pub const DEFAULT_FRACTION: f32 = 0.61;

#[derive(Clone, Copy, Debug)]
pub struct PanelGeometry {
    pub panel: f32,
    pub terminal: f32,
    pub divider: f32,
}

impl PanelGeometry {
    pub fn new(full: f32, fraction: f32, docked: bool, wideness: f32) -> Self {
        let full = full.max(0.);
        let fraction = if fraction.is_finite() {
            fraction.clamp(0.2, 1.)
        } else {
            DEFAULT_FRACTION
        };
        let maximum = if docked {
            (full - DIVIDER_WIDTH - TERMINAL_MIN.min(full * 0.4)).max(0.)
        } else {
            (full - 2. * OVERLAY_MARGIN).max(0.)
        };
        let available = if docked {
            (full - DIVIDER_WIDTH).max(0.)
        } else {
            full
        };
        let narrow = (available * fraction).clamp(PANEL_MIN.min(maximum), maximum);
        // Gömülü panel genişletilince terminal gizlenir; oturumu yaşamaya devam eder.
        let expanded = if docked { full } else { maximum };
        let panel = narrow + (expanded - narrow) * wideness.clamp(0., 1.);
        let divider = if docked && panel < full {
            DIVIDER_WIDTH.min(full - panel)
        } else {
            0.
        };
        Self {
            panel,
            terminal: if docked {
                (full - panel - divider).max(0.)
            } else {
                full
            },
            divider,
        }
    }

    pub fn fraction_at(full: f32, pointer: f32, docked: bool) -> f32 {
        let margin = if docked { 0. } else { OVERLAY_MARGIN };
        let available = if docked { full - DIVIDER_WIDTH } else { full };
        let wanted = (full - margin - pointer) / available.max(1.);
        Self::new(full, wanted, docked, 0.).panel / available.max(1.)
    }
}

// İçerik aynı karede ölçülür; tam ekran geçişinde eski pencere ölçüsü kullanılmaz.
pub struct WorkspaceBody {
    pub terminal: Option<AnyElement>,
    pub browser: Option<Entity<Browser>>,
    pub visible: bool,
    pub docked: bool,
    pub dockness: f32,
    pub rem: Option<Pixels>,
    pub wide: bool,
    pub fraction: f32,
    pub shown: f32,
    pub wideness: f32,
    pub resizing: bool,
    content: Option<AnyElement>,
}

impl WorkspaceBody {
    pub fn new(terminal: AnyElement) -> Self {
        Self {
            terminal: Some(terminal),
            browser: None,
            visible: false,
            docked: false,
            dockness: 0.,
            rem: None,
            wide: false,
            fraction: DEFAULT_FRACTION,
            shown: 0.,
            wideness: 0.,
            resizing: false,
            content: None,
        }
    }
}

impl IntoElement for WorkspaceBody {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for WorkspaceBody {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some("workspace-body".into())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let style = Style {
            size: size(relative(1.).into(), relative(1.).into()),
            ..Default::default()
        };
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let full = f32::from(bounds.size.width);
        let dockness = self.dockness.clamp(0., 1.);
        let floating = PanelGeometry::new(full, self.fraction, false, self.wideness);
        let docking = PanelGeometry::new(full, self.fraction, true, self.wideness);
        // Uzak sürümün geçişi, yerel içerik ölçüsü ve gömülü panel sınırlarıyla hesaplanır.
        let geometry = PanelGeometry {
            panel: floating.panel + (docking.panel - floating.panel) * dockness,
            terminal: full - (full - docking.terminal) * dockness * self.shown.clamp(0., 1.),
            divider: docking.divider * dockness,
        };
        let docked = self.docked && self.visible && dockness >= 0.999 && self.shown >= 0.999;
        let rem = self.rem.unwrap_or(px(theme::REM));
        let mut body = div()
            .id("workspace-content")
            .relative()
            .w(bounds.size.width)
            .h(bounds.size.height)
            .overflow_hidden()
            .bg(rgb(0x000000));
        if let Some(terminal) = self.terminal.take()
            && geometry.terminal > 0.
        {
            body = body.child(
                div()
                    .w(px(geometry.terminal))
                    .h_full()
                    .flex_shrink_0()
                    .overflow_hidden()
                    .child(terminal),
            );
        }
        if let Some(browser) = self.browser.take().filter(|_| self.shown > 0.001) {
            let resize_browser = browser.clone();
            if docked {
                // Terminal ve preview aynı üst ve alt sınırları paylaşır.
                body = body.flex().child(
                    div()
                        .w(px(geometry.panel + geometry.divider))
                        .h_full()
                        .flex_shrink_0()
                        .border_l(px(geometry.divider))
                        .border_color(rgb(theme::BORDER))
                        .overflow_hidden()
                        .child(theme::rem_scope(rem, browser)),
                );
            } else {
                let shift = (1. - self.shown) * (geometry.panel + 2. * OVERLAY_MARGIN);
                body = body.child(
                    div()
                        .absolute()
                        .when(self.visible, |s| s.occlude())
                        .top(px(OVERLAY_MARGIN))
                        .bottom(px(OVERLAY_MARGIN))
                        .right(px(OVERLAY_MARGIN - shift))
                        .w(px(geometry.panel))
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(theme::BORDER))
                        .shadow_xl()
                        .overflow_hidden()
                        .bg(rgb(theme::PANEL))
                        .child(theme::rem_scope(rem, browser)),
                );
            }
            if self.visible && !self.wide && self.resizing {
                let margin = if docked { 0. } else { OVERLAY_MARGIN };
                body = body.child(
                    theme::resize_handle("panel-resize", PanelResize)
                        .top(px(margin))
                        .bottom(px(margin))
                        .left(px(full - margin - geometry.panel - 3.5)),
                );
                body = body.on_drag_move(move |event: &DragMoveEvent<PanelResize>, _, cx| {
                    let fraction = PanelGeometry::fraction_at(
                        full,
                        f32::from(event.event.position.x - bounds.origin.x),
                        docked,
                    );
                    resize_browser.update(cx, |browser, cx| {
                        browser.panel_width = Some(fraction);
                        browser.layout_changed(cx);
                    });
                });
            }
        }
        let mut content = body.into_any_element();
        content.layout_as_root(
            size(
                AvailableSpace::Definite(bounds.size.width),
                AvailableSpace::Definite(bounds.size.height),
            ),
            window,
            cx,
        );
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            content.prepaint_at(bounds.origin, window, cx);
        });
        self.content = Some(content);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(content) = self.content.as_mut() {
                content.paint(window, cx);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_FRACTION, PANEL_MIN, PanelGeometry, TERMINAL_MIN};

    #[test]
    fn docking_reserves_terminal_space_and_overlay_keeps_full_terminal_width() {
        let docked = PanelGeometry::new(1380., DEFAULT_FRACTION, true, 0.);
        assert!((docked.panel / 1379. - 0.61).abs() < 0.0001);
        assert!((docked.terminal / 1379. - 0.39).abs() < 0.0001);
        assert_eq!(docked.terminal + docked.divider + docked.panel, 1380.);
        let overlay = PanelGeometry::new(1380., DEFAULT_FRACTION, false, 0.);
        assert_eq!(overlay.terminal, 1380.);
        assert_eq!(overlay.divider, 0.);
    }

    #[test]
    fn resizing_and_window_changes_never_cover_the_docked_terminal() {
        for full in [0., 100., 550., 900., 1380., 2560.] {
            for fraction in [0., 0.2, 0.61, 1., 5., f32::NAN] {
                let geometry = PanelGeometry::new(full, fraction, true, 0.);
                assert!(geometry.terminal >= 0. && geometry.panel >= 0.);
                assert!(geometry.terminal + geometry.divider + geometry.panel <= full + 0.001);
                if full >= 900. {
                    assert!(geometry.terminal >= TERMINAL_MIN);
                    assert!(geometry.panel >= PANEL_MIN);
                }
            }
        }
        let minimum = PanelGeometry::fraction_at(900., 899., true);
        let maximum = PanelGeometry::fraction_at(900., -500., true);
        assert!((PanelGeometry::new(900., minimum, true, 0.).panel - PANEL_MIN).abs() < 0.001);
        assert!(
            (PanelGeometry::new(900., maximum, true, 0.).terminal - TERMINAL_MIN).abs() < 0.001
        );
    }

    #[test]
    fn expanding_then_restoring_keeps_the_split_fraction() {
        let fraction = PanelGeometry::fraction_at(1380., 450., true);
        let original = PanelGeometry::new(1380., fraction, true, 0.);
        let expanded = PanelGeometry::new(1380., fraction, true, 1.);
        assert_eq!(expanded.panel, 1380.);
        assert_eq!(expanded.terminal, 0.);
        assert_eq!(expanded.divider, 0.);
        let restored = PanelGeometry::new(1380., fraction, true, 0.);
        assert_eq!(restored.terminal, original.terminal);
        assert_eq!(restored.panel, original.panel);
        let resized = PanelGeometry::new(2560., fraction, true, 0.);
        assert!((resized.panel / 2559. - original.panel / 1379.).abs() < 0.0001);
    }
}
