//! Shared colors and small building blocks for Vyber's chrome.
//!
//! The terminal is pure black; panels sit one step above it in neutral greys.
//! Color is reserved for meaning: file icons, syntax, Git state and links.
use gpui::{prelude::*, *};
use gpui_kit::component::{Theme, ThemeMode, tooltip::Tooltip};
use std::sync::Arc;

pub const PANEL: u32 = 0x0b0b0b;
pub const SURFACE: u32 = 0x161616;
pub const HOVER: u32 = 0x1a1a1a;
pub const SELECTED: u32 = 0x232323;
pub const BORDER: u32 = 0x222222;
pub const DIVIDER: u32 = 0x1a1a1a;
pub const TEXT: u32 = 0xe6e6e6;
pub const TEXT_2: u32 = 0xa3a3a3;
pub const MUTED: u32 = 0x6e6e6e;
pub const FAINT: u32 = 0x3a3a3a;
pub const LINK: u32 = 0x5aa2ff;
// Git state colors match the Codex review pane.
pub const ADDED: u32 = 0x40c977;
pub const MODIFIED: u32 = 0xff8549;
pub const DELETED: u32 = 0xfa423e;
pub const CONFLICT: u32 = 0xe4676b;
pub const WARNING_BG: u32 = 0x2a2213;
pub const WARNING: u32 = 0xe8ca8b;

pub fn mono_font() -> &'static str {
    if cfg!(windows) { "Consolas" } else { "Menlo" }
}

pub fn ui_font() -> &'static str {
    if cfg!(windows) {
        "Segoe UI"
    } else {
        ".SystemUIFont"
    }
}

/// The window's rem size: the component library sets it to its font size.
pub const REM: f32 = 13.;

/// `v` pixels at the window's rem size. Inside a [`rem_scope`] the length
/// grows and shrinks with the scope's rem size; elsewhere it is plain pixels.
pub fn rpx(v: f32) -> Rems {
    rems(v / REM)
}

/// Lays out and paints `child` with `rem` as the rem size, so everything in it
/// sized in rems (see [`rpx`]) scales together, like zooming a web page.
pub fn rem_scope(rem: Pixels, child: impl IntoElement) -> RemScope {
    RemScope {
        rem,
        child: child.into_any_element(),
    }
}

pub struct RemScope {
    rem: Pixels,
    child: AnyElement,
}
impl IntoElement for RemScope {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for RemScope {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
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
        let layout = window.with_rem_size(Some(self.rem), |window| {
            self.child.request_layout(window, cx)
        });
        (layout, ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_rem_size(Some(self.rem), |window| self.child.prepaint(window, cx));
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
        window.with_rem_size(Some(self.rem), |window| self.child.paint(window, cx));
    }
}

/// Path of an interface icon embedded by [`crate::icons::Assets`].
pub fn ui(name: &str) -> SharedString {
    format!("vyber/ui/{name}.svg").into()
}

pub fn icon(path: impl Into<SharedString>, color: u32, size: f32) -> Svg {
    svg()
        .path(path)
        .size(rpx(size))
        .flex_shrink_0()
        .text_color(rgb(color))
}

/// A square, icon-only button with a tooltip.
pub fn icon_button(
    id: impl Into<ElementId>,
    name: &str,
    tooltip: impl Into<SharedString>,
) -> Stateful<Div> {
    let id = id.into();
    let group: SharedString = format!("icon-button-{id}").into();
    let tooltip = tooltip.into();
    div()
        .id(id)
        .group(group.clone())
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(26.))
        .flex_shrink_0()
        .rounded_md()
        .cursor_pointer()
        .hover(|s| s.bg(rgb(HOVER)))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(
            icon(ui(name), TEXT_2, 15.).group_hover(group, |s| s.text_color(rgb(TEXT))),
        )
}

/// A borderless text row used in menus.
pub fn chip(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .rounded_md()
        .text_size(rpx(12.))
        .text_color(rgb(0xc8c8c8))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(HOVER)).text_color(rgb(TEXT)))
        .child(text.into())
}

/// A compact text button, optionally with a leading icon.
pub fn text_button(
    id: impl Into<ElementId>,
    leading: Option<&str>,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap_1p5()
        .h(rpx(26.))
        .px_2()
        .rounded_md()
        .text_size(rpx(12.))
        .text_color(rgb(TEXT_2))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(HOVER)).text_color(rgb(TEXT)))
        .when_some(leading, |s, name| s.child(icon(ui(name), TEXT_2, 14.)))
        .child(label.into())
}

/// A menu or popover fading in as it opens.
pub fn menu_in<E: IntoElement + 'static>(id: &'static str, element: E) -> impl IntoElement {
    div().child(element).with_animation(
        id,
        Animation::new(std::time::Duration::from_millis(130)).with_easing(ease_out_quint()),
        |el, t| el.opacity(t),
    )
}

/// Ease-out curve for panel slides: quick start, soft landing.
pub fn ease_out(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

/// A thin grab strip on the left edge of a panel. Dragging it starts a drag
/// carrying `value`; the owner resizes on `DragMoveEvent`s of that type.
pub fn resize_handle<T: Clone + Render>(id: &'static str, value: T) -> Stateful<Div> {
    div()
        .id(id)
        .group(id)
        .absolute()
        .occlude()
        .top_0()
        .bottom_0()
        .w(px(7.))
        .flex()
        .justify_center()
        .cursor_col_resize()
        .child(
            div()
                .w(px(2.))
                .h_full()
                .bg(rgb(0x4a4a4a))
                .opacity(0.)
                .group_hover(id, |s| s.opacity(1.)),
        )
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_drag(value, |value, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| value.clone())
        })
}

/// Applies the Vyber palette to the component library (inputs, menus,
/// markdown, editor and syntax colors).
pub fn apply(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        theme.background = rgb(0x000000).into();
        theme.foreground = rgb(TEXT).into();
        theme.title_bar = rgb(0x000000).into();
        theme.title_bar_border = rgb(0x242424).into();
        theme.border = rgb(BORDER).into();
        theme.input = rgb(BORDER).into();
        theme.ring = rgb(0x3a3a3a).into();
        theme.muted = rgb(0x141414).into();
        theme.muted_foreground = rgb(MUTED).into();
        theme.accent = rgb(0x1c1c1c).into();
        theme.accent_foreground = rgb(TEXT).into();
        theme.secondary = rgb(SURFACE).into();
        theme.secondary_hover = rgb(HOVER).into();
        theme.popover = rgb(0x121212).into();
        theme.popover_foreground = rgb(TEXT).into();
        theme.list_hover = rgb(HOVER).into();
        theme.list_active = rgb(SELECTED).into();
        theme.list_active_border = rgb(SELECTED).into();
        theme.selection = rgba(0x3b82f655).into();
        theme.caret = rgb(TEXT).into();
        theme.link = rgb(LINK).into();
        theme.link_hover = rgb(0x8cbcff).into();
        theme.link_active = rgb(0x8cbcff).into();
        theme.scrollbar = rgba(0x00000000).into();
        theme.scrollbar_thumb = rgba(0xffffff26).into();
        theme.scrollbar_thumb_hover = rgba(0xffffff40).into();
        theme.table_head = rgb(0x141414).into();
        theme.table_head_foreground = rgb(TEXT).into();
        theme.table_row_border = rgb(BORDER).into();
        theme.font_size = px(REM);
        theme.mono_font_size = px(13.);
        if let Ok(highlight) = serde_json::from_str(HIGHLIGHT) {
            theme.highlight_theme = Arc::new(highlight);
        }
    });
}

/// Syntax colors tuned for the near-black editor surface (Zed theme format).
const HIGHLIGHT: &str = r##"{
  "name": "Vyber Night",
  "appearance": "dark",
  "style": {
    "editor.background": "#0b0b0b",
    "editor.foreground": "#d4d4d4",
    "editor.active_line.background": "#141414",
    "editor.line_number": "#454545",
    "editor.active_line_number": "#a3a3a3",
    "editor.invisible": "#3a3a3a",
    "conflict": "#e4676b",
    "created": "#40c977",
    "created.background": "#12261c",
    "deleted.background": "#2a1417",
    "error.background": "#2a1417",
    "error.border": "#e4676b",
    "hidden": "#6e6e6e",
    "hint": "#8b93a6",
    "hint.background": "#15171d",
    "hint.border": "#2a2f3a",
    "info.background": "#10223a",
    "info.border": "#3b82f6",
    "modified": "#ff8549",
    "modified.background": "#2a2213",
    "predictive": "#5c5c5c",
    "success.background": "#12261c",
    "warning.background": "#2a2213",
    "warning.border": "#8a6d1f",
    "syntax": {
      "attribute": { "color": "#d19a66" },
      "boolean": { "color": "#d19a66" },
      "comment": { "color": "#6a737d", "font_style": "italic" },
      "comment.doc": { "color": "#7d8590", "font_style": "italic" },
      "constant": { "color": "#d19a66" },
      "constructor": { "color": "#e5c07b" },
      "embedded": { "color": "#d4d4d4" },
      "emphasis": { "font_style": "italic" },
      "emphasis.strong": { "font_weight": 700 },
      "enum": { "color": "#e5c07b" },
      "function": { "color": "#61afef" },
      "keyword": { "color": "#c678dd" },
      "label": { "color": "#e06c75" },
      "link_text": { "color": "#5aa2ff" },
      "link_uri": { "color": "#56b6c2", "font_style": "italic" },
      "number": { "color": "#d19a66" },
      "operator": { "color": "#56b6c2" },
      "preproc": { "color": "#c678dd" },
      "property": { "color": "#e06c75" },
      "punctuation": { "color": "#8b919c" },
      "punctuation.bracket": { "color": "#a0a6b1" },
      "punctuation.delimiter": { "color": "#8b919c" },
      "punctuation.list_marker": { "color": "#e06c75" },
      "punctuation.special": { "color": "#c678dd" },
      "string": { "color": "#98c379" },
      "string.escape": { "color": "#56b6c2" },
      "string.regex": { "color": "#56b6c2" },
      "string.special": { "color": "#98c379" },
      "string.special.symbol": { "color": "#56b6c2" },
      "tag": { "color": "#e06c75" },
      "tag.doctype": { "color": "#c678dd" },
      "text.code.span": { "color": "#98c379" },
      "text.literal": { "color": "#98c379" },
      "title": { "color": "#e5c07b", "font_weight": 600 },
      "type": { "color": "#e5c07b" },
      "variable": { "color": "#d4d4d4" },
      "variable.special": { "color": "#e06c75" },
      "variant": { "color": "#d19a66" }
    }
  }
}"##;

#[cfg(test)]
mod tests {
    #[test]
    fn highlight_theme_parses() {
        serde_json::from_str::<gpui_kit::component::highlighter::HighlightTheme>(super::HIGHLIGHT)
            .unwrap();
    }
}
