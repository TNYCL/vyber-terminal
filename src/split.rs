//! Size a split from its saved ratio using its actual, possibly nested bounds.
use gpui::{prelude::*, *};
use gpui_kit::component::resizable::{h_resizable, resizable_panel, v_resizable};
use std::rc::Rc;

type ResizeCallback = Rc<dyn Fn(&f32, &mut Window, &mut App)>;

pub struct SplitPane {
    anchor: usize,
    vertical: bool,
    ratio: f32,
    children: Option<(AnyElement, AnyElement)>,
    on_resize: ResizeCallback,
    content: Option<AnyElement>,
}

impl SplitPane {
    pub fn new(
        anchor: usize,
        vertical: bool,
        ratio: f32,
        first: AnyElement,
        second: AnyElement,
    ) -> Self {
        Self {
            anchor,
            vertical,
            ratio,
            children: Some((first, second)),
            on_resize: Rc::new(|_, _, _| {}),
            content: None,
        }
    }

    pub fn on_resize(mut self, callback: impl Fn(&f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_resize = Rc::new(callback);
        self
    }
}

impl IntoElement for SplitPane {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for SplitPane {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(("terminal-split-layout", self.anchor).into())
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
        let Some((first, second)) = self.children.take() else {
            return;
        };
        let total = if self.vertical {
            bounds.size.height
        } else {
            bounds.size.width
        };
        let ratio = if self.ratio.is_finite() {
            self.ratio.clamp(0., 1.)
        } else {
            0.5
        };
        let minimum = px(100.).min(total / 2.);
        let first_size = (total * ratio).clamp(minimum, total - minimum);
        let group = if self.vertical {
            v_resizable(("terminal-split", self.anchor))
        } else {
            h_resizable(("terminal-split", self.anchor))
        };
        let callback = self.on_resize.clone();
        let mut content = group
            .child(
                resizable_panel()
                    .size(first_size)
                    .size_range(px(100.)..px(6000.))
                    .child(first),
            )
            .child(
                resizable_panel()
                    .size(total - first_size)
                    .size_range(px(100.)..px(6000.))
                    .child(second),
            )
            .on_resize(move |state, window, cx| {
                let sizes = state.read(cx).sizes();
                let total = sizes.iter().map(|size| f32::from(*size)).sum::<f32>();
                if sizes.len() == 2 && total.is_finite() && total > 0. {
                    callback(&(f32::from(sizes[0]) / total), window, cx);
                }
            })
            .into_any_element();
        content.layout_as_root(
            size(
                AvailableSpace::Definite(bounds.size.width),
                AvailableSpace::Definite(bounds.size.height),
            ),
            window,
            cx,
        );
        content.prepaint_at(bounds.origin, window, cx);
        self.content = Some(content);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(content) = self.content.as_mut() {
            content.paint(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SplitPane;
    use crate::layout::Layout;
    use gpui::{
        AnyElement, App, AppContext, Bounds, Context, IntoElement, Point, Render, TestAppContext,
        Window, WindowBounds, WindowOptions, div, point, prelude::*, px, size,
    };
    use gpui_kit::test::{TestSupportExt, TestWindowExt};

    struct Harness {
        tabs: Vec<Layout>,
        tab: usize,
        zoomed: bool,
    }

    impl Harness {
        fn layout(&self, layout: &Layout, cx: &mut Context<Self>) -> AnyElement {
            match layout {
                Layout::Leaf(id) => div()
                    .id(("split-test-pane", *id))
                    .test_support()
                    .size_full()
                    .into_any_element(),
                Layout::Split {
                    vertical,
                    ratio,
                    first,
                    second,
                    ..
                } => {
                    let anchor = second.first();
                    SplitPane::new(
                        anchor,
                        *vertical,
                        *ratio,
                        self.layout(first, cx),
                        self.layout(second, cx),
                    )
                    .on_resize(cx.listener(move |this, ratio: &f32, _, cx| {
                        assert!(this.tabs[this.tab].resize_split(anchor, *ratio));
                        cx.notify();
                    }))
                    .into_any_element()
                }
            }
        }
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            if self.zoomed {
                self.layout(&Layout::Leaf(3), cx)
            } else {
                self.layout(&self.tabs[self.tab].clone(), cx)
            }
        }
    }

    fn pane_sizes(window: &Window, ids: &[usize], vertical: bool) -> Vec<f32> {
        ids.iter()
            .map(|id| {
                let bounds = window.find(("split-test-pane", *id)).bounds();
                f32::from(if vertical {
                    bounds.size.height
                } else {
                    bounds.size.width
                })
            })
            .collect()
    }

    fn assert_sizes(actual: &[f32], expected: &[f32]) {
        for (actual, expected) in actual.iter().zip(expected) {
            assert!((actual - expected).abs() < 1., "{actual} != {expected}");
        }
    }

    fn drag_divider(window: &mut Window, pane: usize, target: f32, vertical: bool, cx: &mut App) {
        let bounds = window.find(("split-test-pane", pane)).bounds();
        let (from, to) = if vertical {
            (
                point(bounds.center().x, bounds.top() - px(2.)),
                point(bounds.center().x, px(target)),
            )
        } else {
            (
                point(bounds.left() - px(2.), bounds.center().y),
                point(px(target), bounds.center().y),
            )
        };
        window.drag(from, to, cx);
        window.render_frame(cx);
    }

    fn check_tab_switches_and_restore(cx: &mut TestAppContext, vertical: bool) {
        let mut first = Layout::Leaf(1);
        first.split(1, 2, vertical);
        first.split(2, 3, vertical);
        let mut second = Layout::Leaf(4);
        second.split(4, 5, vertical);
        // Exercise colliding legacy keys across tabs as well as nested splits.
        if let Layout::Split { key, .. } = &mut second {
            *key = 3;
        }
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(1200.), px(800.)),
                    })),
                    ..Default::default()
                },
                cx,
                |_, cx| {
                    cx.new(|_| Harness {
                        tabs: vec![first, second],
                        tab: 0,
                        zoomed: false,
                    })
                },
            )
            .unwrap()
        });
        let total = if vertical { 800. } else { 1200. };
        let expected = [total * 0.6, total * 0.4 - 120., 120.];
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            drag_divider(window, 2, total * 0.6, vertical, cx);
            drag_divider(window, 3, total - 120., vertical, cx);
            assert_sizes(&pane_sizes(window, &[1, 2, 3], vertical), &expected);
            for _ in 0..4 {
                view.update(cx, |view, cx| {
                    view.tab = 1;
                    cx.notify();
                });
                window.render_frame(cx);
                drag_divider(window, 5, total * 0.35, vertical, cx);
                view.update(cx, |view, cx| {
                    view.tab = 0;
                    cx.notify();
                });
                window.render_frame(cx);
                assert_sizes(&pane_sizes(window, &[1, 2, 3], vertical), &expected);
            }
            view.update(cx, |view, cx| {
                view.zoomed = true;
                cx.notify();
            });
            window.render_frame(cx);
            assert_sizes(&pane_sizes(window, &[3], vertical), &[total]);
            view.update(cx, |view, cx| {
                view.zoomed = false;
                cx.notify();
            });
            window.render_frame(cx);
            assert_sizes(&pane_sizes(window, &[1, 2, 3], vertical), &expected);
        })
        .unwrap();

        cx.simulate_window_resize(handle, size(px(1800.), px(1000.)));
        let scale = if vertical { 1000. / 800. } else { 1.5 };
        let resized: Vec<_> = expected.iter().map(|size| size * scale).collect();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert_sizes(&pane_sizes(window, &[1, 2, 3], vertical), &resized);
            let saved = serde_json::to_vec(&view.read(cx).tabs).unwrap();
            view.update(cx, |view, cx| {
                view.tab = 1;
                cx.notify();
            });
            window.render_frame(cx);
            view.update(cx, |view, cx| {
                view.tabs = serde_json::from_slice(&saved).unwrap();
                view.tab = 0;
                cx.notify();
            });
            window.render_frame(cx);
            assert_sizes(&pane_sizes(window, &[1, 2, 3], vertical), &resized);
        })
        .unwrap();
    }

    #[gpui::test]
    fn narrow_right_terminal_survives_tab_switches_zoom_resize_and_restore(
        cx: &mut TestAppContext,
    ) {
        check_tab_switches_and_restore(cx, false);
    }

    #[gpui::test]
    fn short_bottom_terminal_survives_tab_switches_zoom_resize_and_restore(
        cx: &mut TestAppContext,
    ) {
        check_tab_switches_and_restore(cx, true);
    }
}
