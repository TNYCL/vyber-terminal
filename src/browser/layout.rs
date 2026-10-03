//! Measure sidebars against the panel they actually occupy, including splits.
use super::SidebarResize;
use crate::theme;
use gpui::{prelude::*, *};
use std::rc::Rc;

const DOCUMENT_MIN: f32 = 288.;
const SIDEBAR_MIN: f32 = 190.;

fn fitted_width(full: f32, wanted: f32, scale: f32) -> f32 {
    let full = full.max(0.);
    let scale = scale.max(0.1);
    let document = (DOCUMENT_MIN * scale).min(full * 0.6);
    let maximum = (full - document).max(0.);
    let minimum = (SIDEBAR_MIN * scale).min(maximum);
    let wanted = if wanted.is_finite() { wanted } else { minimum };
    wanted.clamp(minimum, maximum)
}

type Resize = Rc<dyn Fn(f32, &mut Window, &mut App)>;

pub(super) struct SidebarLayout {
    main: Option<AnyElement>,
    sidebar: Option<AnyElement>,
    wanted: f32,
    shown: f32,
    scale: f32,
    resize: Option<Resize>,
    content: Option<AnyElement>,
}

impl SidebarLayout {
    pub(super) fn new(
        main: AnyElement,
        sidebar: Option<AnyElement>,
        wanted: f32,
        shown: f32,
        scale: f32,
    ) -> Self {
        let scale = if scale.is_finite() {
            scale.max(0.1)
        } else {
            1.
        };
        Self {
            main: Some(main),
            sidebar,
            wanted,
            shown,
            scale,
            resize: None,
            content: None,
        }
    }

    pub(super) fn on_resize(
        mut self,
        resize: impl Fn(f32, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.resize = Some(Rc::new(resize));
        self
    }
}

impl IntoElement for SidebarLayout {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for SidebarLayout {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some("browser-main".into())
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
        (
            window.request_layout(
                Style {
                    size: size(relative(1.).into(), relative(1.).into()),
                    ..Default::default()
                },
                None,
                cx,
            ),
            (),
        )
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
        let width = if self.sidebar.is_some() {
            fitted_width(full, self.wanted, self.scale)
        } else {
            0.
        };
        let visible = width * self.shown.clamp(0., 1.);
        let mut body = div()
            .relative()
            .flex()
            .w(bounds.size.width)
            .h(bounds.size.height);
        if let Some(main) = self.main.take() {
            body = body.child(
                div()
                    .w(px((full - visible).max(0.)))
                    .h_full()
                    .flex_shrink_0()
                    .min_w_0()
                    .child(main),
            );
        }
        if let Some(sidebar) = self.sidebar.take() {
            body = body.child(
                div()
                    .relative()
                    .w(px(visible))
                    .h_full()
                    .flex_shrink_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left_0()
                            .w(px(width))
                            .child(sidebar),
                    )
                    .child(theme::resize_handle("sidebar-resize", SidebarResize).left_0()),
            );
        }
        if let Some(resize) = self.resize.clone() {
            let scale = self.scale;
            body = body.on_drag_move(move |event: &DragMoveEvent<SidebarResize>, window, cx| {
                let wanted = f32::from(bounds.right() - event.event.position.x);
                resize(fitted_width(full, wanted, scale) / scale, window, cx);
            });
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
    use super::{SidebarLayout, fitted_width};
    use gpui::{
        Bounds, Context, IntoElement, Point, Render, TestAppContext, Window, WindowBounds,
        WindowOptions, div, prelude::*, px, size,
    };
    use gpui_kit::test::TestSupportExt;
    use gpui_kit::test::TestWindowExt;

    struct Harness {
        width: f32,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(self.width))
                .h_full()
                .flex()
                .flex_col()
                .child(div().h(px(42.)).flex_shrink_0())
                .child(
                    div().flex_1().min_h_0().child(SidebarLayout::new(
                        div()
                            .id("layout-document")
                            .size_full()
                            .test_support()
                            .into_any_element(),
                        Some(
                            div()
                                .id("layout-sidebar")
                                .size_full()
                                .test_support()
                                .into_any_element(),
                        ),
                        460.,
                        1.,
                        1.,
                    )),
                )
        }
    }

    #[test]
    fn small_panels_keep_document_space_and_restore_the_requested_sidebar() {
        assert_eq!(fitted_width(480., 460., 1.), 192.);
        assert_eq!(fitted_width(900., 460., 1.), 460.);
        assert_eq!(fitted_width(480., 260., 1.), 192.);
        assert_eq!(fitted_width(900., 260., 1.), 260.);
        assert_eq!(fitted_width(960., 920., 2.), 384.);
        for full in [0., 50., 200., 480., 900.] {
            for wanted in [0., 260., 460., 2000., f32::NAN] {
                let sidebar = fitted_width(full, wanted, 1.);
                assert!(sidebar >= 0. && sidebar <= full);
                assert!(full - sidebar >= 288_f32.min(full * 0.6));
            }
        }
    }

    #[gpui_kit::test]
    fn rendered_sidebar_fits_the_panel_and_recovers_after_resize(cx: &mut TestAppContext) {
        let (handle, harness) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(900.), px(240.)),
                    })),
                    ..Default::default()
                },
                cx,
                |_, cx| cx.new(|_| Harness { width: 480. }),
            )
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let document = window.find("layout-document").bounds();
            let sidebar = window.find("layout-sidebar").bounds();
            assert_eq!(document.size.width, px(288.));
            assert_eq!(sidebar.size.width, px(192.));
            assert_eq!(sidebar.right(), px(480.));
            assert_eq!(sidebar.top(), px(42.));
            assert_eq!(sidebar.bottom(), px(240.));
            harness.update(cx, |harness, cx| {
                harness.width = 900.;
                cx.notify();
            });
            window.render_frame(cx);
            assert_eq!(window.find("layout-document").bounds().size.width, px(440.));
            assert_eq!(window.find("layout-sidebar").bounds().size.width, px(460.));
            assert_eq!(window.find("layout-sidebar").bounds().right(), px(900.));
        })
        .unwrap();
    }
}
