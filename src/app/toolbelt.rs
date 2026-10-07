//! The toolbelt beside the terminals, after iTerm2's: Jobs shows the process
//! tree of the focused terminal, Session Status what every Claude Code and
//! Codex session is doing.
use super::{ToggleToolbelt, Vyber};
use crate::{
    agents::{self, Activity, AgentKind, AgentState, Reason},
    config::{Config, ToolbeltSide},
    platform::Signal,
    theme::{self, *},
};
use gpui::{prelude::*, *};
use gpui_kit::component::tooltip::Tooltip;
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

const HEADER: f32 = 30.;
const JOB_ROW: f32 = 22.;
/// Narrower windows leave the toolbelt out to keep room for the terminals.
const WINDOW_MIN: f32 = 640.;
/// The terminals keep at least this much of the window.
const TERMINAL_MIN: f32 = 320.;

/// Dragging the toolbelt's inner edge.
#[derive(Clone)]
pub(crate) struct ToolbeltResize;
impl Render for ToolbeltResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Dragging the line between Jobs and Session Status.
#[derive(Clone)]
pub(crate) struct ToolbeltSplit;
impl Render for ToolbeltSplit {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// What the agent in one terminal is doing, as of the last poll.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct AgentStatus {
    pub kind: AgentKind,
    pub pid: u32,
    pub state: AgentState,
    /// What a waiting agent waits for.
    pub detail: Option<String>,
    /// When it started working or waiting, or when it finished.
    pub since: Option<Instant>,
    /// The name the user gave the session.
    pub name: Option<String>,
}

pub(super) struct Toolbelt {
    motion: Motion,
    /// Processes whose children Jobs hides.
    collapsed: HashSet<u32>,
    selected: Option<u32>,
    signal: Signal,
    signal_menu: bool,
    jobs_open: bool,
    status_open: bool,
    /// The width and split while their edge is dragged, saved on release.
    width: Option<f32>,
    split: Option<f32>,
    jobs_scroll: ScrollHandle,
    status_scroll: ScrollHandle,
}

impl Toolbelt {
    pub fn new(config: &Config) -> Self {
        Self {
            motion: Motion::settled(config.toolbelt, config.toolbelt_side),
            collapsed: HashSet::new(),
            selected: None,
            signal: Signal::Terminate,
            signal_menu: false,
            jobs_open: true,
            status_open: true,
            width: None,
            split: None,
            jobs_scroll: ScrollHandle::new(),
            status_scroll: ScrollHandle::new(),
        }
    }
    /// The toolbelt's edge is being dragged.
    pub fn resizing(&self) -> bool {
        self.width.is_some()
    }
    #[cfg_attr(
        test,
        allow(dead_code, reason = "Process scanning is disabled in UI tests.")
    )]
    pub fn shown(&self) -> bool {
        self.motion.shown
    }
}

/// The toolbelt's slide in and out, like the file panel's.
#[derive(Clone, Copy)]
struct Motion {
    shown: bool,
    side: ToolbeltSide,
    from: f32,
    start: Instant,
}
impl Motion {
    const DURATION: f32 = 0.24;
    fn settled(shown: bool, side: ToolbeltSide) -> Self {
        Self {
            shown,
            side,
            from: unit(shown),
            start: Instant::now() - Duration::from_secs(1),
        }
    }
    fn progress(&self) -> f32 {
        (self.start.elapsed().as_secs_f32() / Self::DURATION).min(1.)
    }
    fn running(&self) -> bool {
        self.progress() < 1.
    }
    fn value(&self) -> f32 {
        self.from + (unit(self.shown) - self.from) * theme::ease_out(self.progress())
    }
}
fn unit(on: bool) -> f32 {
    f32::from(u8::from(on))
}

pub(super) fn state_color(state: AgentState) -> u32 {
    match state {
        AgentState::Working => MODIFIED,
        AgentState::Waiting => WAITING,
        AgentState::Responded => ADDED,
        AgentState::Idle => FAINT,
    }
}

fn state_label(state: AgentState) -> &'static str {
    match state {
        AgentState::Working => "Working",
        AgentState::Waiting => "Waiting",
        AgentState::Responded => "Responded",
        AgentState::Idle => "Idle",
    }
}

pub(super) fn state_dot(state: AgentState, size: f32) -> Div {
    div()
        .size(px(size))
        .flex_shrink_0()
        .rounded_full()
        .bg(rgb(state_color(state)))
}

impl Vyber {
    pub(super) fn toggle_toolbelt(
        &mut self,
        _: &ToggleToolbelt,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_shortcuts = false;
        Config::update(cx, |c| c.toolbelt = !c.toolbelt);
        cx.notify();
    }

    /// Reads what each terminal's agent does now from the latest process
    /// scan, its turns and its screen. Returns whether anything changed.
    pub(super) fn refresh_agents(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        let now = Instant::now();
        let looking_at = window
            .is_window_active()
            .then_some(self.active)
            .filter(|id| self.tabs.get(self.tab).is_some_and(|l| l.contains(*id)));
        let mut changed = false;
        for (id, slot) in &mut self.slots {
            let terminal = slot.terminal.read(cx);
            let found = terminal.shell.and_then(|shell| self.scan.agent(shell));
            let status = found.map(|(kind, pid)| {
                let claude = (kind == AgentKind::Claude)
                    .then(|| self.scan.claude.get(&pid))
                    .flatten();
                let turn_active = slot.turn.as_ref().is_some_and(|t| t.active);
                let working = match claude.map(|c| c.status.as_str()) {
                    Some("busy") => true,
                    Some("idle" | "waiting") => false,
                    _ => turn_active,
                };
                let prompt = working && agents::approval_prompt(&terminal.screen_text(16));
                let attention = slot
                    .attention
                    .is_some_and(|at| terminal.last_input().is_none_or(|input| input < at));
                let activity = agents::activity(claude, turn_active, prompt, attention);
                let state = slot
                    .tracker
                    .update(pid, activity, looking_at == Some(*id), now);
                let detail = match activity {
                    Activity::Waiting(Reason::Claude) => Some(
                        claude
                            .and_then(|c| c.waiting_for.clone())
                            .unwrap_or_else(|| "input".into()),
                    ),
                    Activity::Waiting(Reason::Approval) => Some("approval".into()),
                    Activity::Waiting(Reason::Attention) => Some("attention".into()),
                    _ => None,
                };
                AgentStatus {
                    kind,
                    pid,
                    state,
                    detail,
                    since: match state {
                        AgentState::Working | AgentState::Waiting => slot.tracker.since(),
                        AgentState::Responded | AgentState::Idle => slot.tracker.finished(),
                    },
                    name: claude.and_then(|c| c.name.clone()),
                }
            });
            if status.is_none() {
                slot.tracker = agents::Tracker::default();
            }
            if slot.agent != status {
                slot.agent = status;
                changed = true;
            }
        }
        changed
    }

    /// How far the toolbelt is shown, 0–1, and whether it is sliding.
    pub(super) fn toolbelt_progress(&mut self, full: f32, cx: &App) -> (f32, bool) {
        let config = cx.global::<Config>();
        let shown = config.toolbelt && full >= WINDOW_MIN;
        let side = config.toolbelt_side;
        let motion = self.toolbelt.motion;
        if motion.shown != shown || motion.side != side {
            self.toolbelt.motion = if cx.reduce_motion() {
                Motion::settled(shown, side)
            } else {
                Motion {
                    shown,
                    side,
                    // Moving to the other edge slides in there afresh.
                    from: if motion.side == side {
                        motion.value()
                    } else {
                        0.
                    },
                    start: Instant::now(),
                }
            };
        }
        let motion = self.toolbelt.motion;
        (motion.value(), motion.running())
    }

    /// Puts the toolbelt beside `body`, `shown` of the way in.
    pub(super) fn with_toolbelt(
        &mut self,
        body: impl IntoElement,
        shown: f32,
        full: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // A finished drag saves the new size to config.toml.
        if !cx.has_active_drag() {
            if let Some(width) = self.toolbelt.width.take() {
                Config::update(cx, |c| c.toolbelt_width = width);
            }
            if let Some(split) = self.toolbelt.split.take() {
                Config::update(cx, |c| c.toolbelt_split = f64::from(split));
            }
        }
        let body = div().flex_1().min_w_0().h_full().child(body);
        if shown <= 0.001 {
            return body.into_any_element();
        }
        let config = cx.global::<Config>();
        let side = self.toolbelt.motion.side;
        let width = self
            .toolbelt
            .width
            .unwrap_or(config.toolbelt_width)
            .min((full - TERMINAL_MIN).max(200.));
        let belt = self.toolbelt_panel(width, shown, side, cx);
        div()
            .size_full()
            .flex()
            .on_drag_move(
                cx.listener(move |this, e: &DragMoveEvent<ToolbeltResize>, _, cx| {
                    let x = e.event.position.x;
                    let width = match side {
                        ToolbeltSide::Right => e.bounds.right() - x,
                        ToolbeltSide::Left => x - e.bounds.left(),
                    };
                    this.toolbelt.width = Some(f32::from(width).clamp(200., 640.));
                    cx.notify();
                }),
            )
            .map(|row| match side {
                ToolbeltSide::Left => row.child(belt).child(body),
                ToolbeltSide::Right => row.child(body).child(belt),
            })
            .into_any_element()
    }

    fn toolbelt_panel(
        &mut self,
        width: f32,
        shown: f32,
        side: ToolbeltSide,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let split = self
            .toolbelt
            .split
            .unwrap_or(cx.global::<Config>().toolbelt_split as f32);
        let (jobs_open, status_open) = (self.toolbelt.jobs_open, self.toolbelt.status_open);
        let jobs = self.jobs_section(cx);
        let status = self.status_section(cx);
        let content = div()
            .absolute()
            .top_0()
            .bottom_0()
            .map(|s| match side {
                ToolbeltSide::Right => s.left_0(),
                ToolbeltSide::Left => s.right_0(),
            })
            .w(px(width))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .overflow_hidden()
                    .map(|s| match (jobs_open, status_open) {
                        (true, true) => s.h(relative(split)).flex_shrink_0(),
                        (true, false) => s.flex_1(),
                        (false, _) => s.flex_shrink_0(),
                    })
                    .child(jobs),
            )
            .child(if jobs_open && status_open {
                div()
                    .id("toolbelt-split")
                    .group("toolbelt-split")
                    .h(px(5.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .cursor_row_resize()
                    .child(
                        div()
                            .h(px(1.))
                            .w_full()
                            .bg(rgb(BORDER))
                            .group_hover("toolbelt-split", |s| s.h(px(2.)).bg(rgb(0x4a4a4a))),
                    )
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_drag(ToolbeltSplit, |value, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| value.clone())
                    })
            } else {
                div()
                    .id("toolbelt-split")
                    .h(px(1.))
                    .flex_shrink_0()
                    .bg(rgb(BORDER))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .overflow_hidden()
                    .map(|s| {
                        if status_open {
                            s.flex_1()
                        } else {
                            s.flex_shrink_0()
                        }
                    })
                    .child(status),
            )
            .on_drag_move(
                cx.listener(|this, e: &DragMoveEvent<ToolbeltSplit>, _, cx| {
                    let height = f32::from(e.bounds.size.height).max(1.);
                    let y = f32::from(e.event.position.y - e.bounds.top());
                    this.toolbelt.split = Some((y / height).clamp(0.15, 0.85));
                    cx.notify();
                }),
            );
        div()
            .id("toolbelt")
            .relative()
            .h_full()
            .w(px(width * shown))
            .flex_shrink_0()
            .overflow_hidden()
            .bg(rgb(PANEL))
            .map(|s| match side {
                ToolbeltSide::Right => s.border_l_1(),
                ToolbeltSide::Left => s.border_r_1(),
            })
            .border_color(rgb(BORDER))
            .opacity(0.4 + 0.6 * shown)
            .text_size(px(12.))
            .child(content)
            .when(shown >= 0.999, |s| {
                s.child(theme::resize_handle("toolbelt-resize", ToolbeltResize).map(
                    |h| match side {
                        ToolbeltSide::Right => h.left_0(),
                        ToolbeltSide::Left => h.right_0(),
                    },
                ))
            })
            .into_any_element()
    }

    fn section_header(
        &self,
        id: &'static str,
        title: &'static str,
        open: bool,
        count: Option<usize>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(HEADER))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1p5()
            .pl_2()
            .pr_1()
            .cursor_pointer()
            .child(theme::chevron(unit(open), MUTED, 13.))
            .child(theme::section_title(title))
            .when_some(count.filter(|n| *n > 0), |s, n| {
                s.child(theme::count_badge(n))
            })
            .child(div().flex_1())
            .on_click(cx.listener(move |this, _, _, cx| {
                match id {
                    "toolbelt-jobs-header" => this.toolbelt.jobs_open = !this.toolbelt.jobs_open,
                    _ => this.toolbelt.status_open = !this.toolbelt.status_open,
                }
                cx.notify();
            }))
    }

    fn jobs_section(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let shell = self
            .slots
            .get(&self.active)
            .and_then(|s| s.terminal.read(cx).shell);
        let jobs = shell
            .map(|shell| self.scan.jobs(shell, &self.toolbelt.collapsed))
            .unwrap_or_default();
        if self
            .toolbelt
            .selected
            .is_some_and(|pid| !jobs.iter().any(|j| j.pid == pid))
        {
            self.toolbelt.selected = None;
            self.toolbelt.signal_menu = false;
        }
        let other = match self.toolbelt.motion.side {
            ToolbeltSide::Right => ToolbeltSide::Left,
            ToolbeltSide::Left => ToolbeltSide::Right,
        };
        let header = self
            .section_header(
                "toolbelt-jobs-header",
                "JOBS",
                self.toolbelt.jobs_open,
                None,
                cx,
            )
            .child(
                theme::icon_button(
                    "toolbelt-move",
                    match other {
                        ToolbeltSide::Left => "panel-left",
                        ToolbeltSide::Right => "panel-right",
                    },
                    match other {
                        ToolbeltSide::Left => "Move to the left",
                        ToolbeltSide::Right => "Move to the right",
                    },
                )
                .size(px(24.))
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    Config::update(cx, |c| c.toolbelt_side = other);
                })),
            )
            .child(
                theme::icon_button(
                    "toolbelt-close",
                    "x",
                    if cfg!(target_os = "macos") {
                        "Hide toolbelt  (⌘J)"
                    } else {
                        "Hide toolbelt  (Ctrl+Shift+J)"
                    },
                )
                .size(px(24.))
                .on_click(cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.toggle_toolbelt(&ToggleToolbelt, window, cx);
                })),
            );
        if !self.toolbelt.jobs_open {
            return header.into_any_element();
        }
        let home = dirs::home_dir();
        let empty = if shell.is_none() {
            Some("No terminal")
        } else if jobs.is_empty() {
            Some("Process information unavailable")
        } else {
            None
        };
        let count = jobs.len();
        let rows = jobs
            .into_iter()
            .map(|job| {
                let pid = job.pid;
                let label = agents::display_command(&job.name, &job.command, home.as_deref());
                let selected = self.toolbelt.selected == Some(pid);
                let open = !self.toolbelt.collapsed.contains(&pid);
                let tooltip: SharedString = format!("{label}\nPID {pid}").into();
                div()
                    .id(("toolbelt-job", pid))
                    .h(px(JOB_ROW))
                    .flex_shrink_0()
                    .mx_1()
                    .pl(px(4. + 12. * job.depth.min(12) as f32))
                    .pr(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .rounded(px(5.))
                    .text_color(rgb(if selected || job.agent.is_some() {
                        TEXT
                    } else {
                        TEXT_2
                    }))
                    .when(selected, |s| s.bg(rgb(SELECTED)))
                    .when(!selected, |s| s.hover(|s| s.bg(rgb(HOVER))))
                    .cursor_pointer()
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toolbelt.selected =
                            (this.toolbelt.selected != Some(pid)).then_some(pid);
                        this.toolbelt.signal_menu = false;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .id(("toolbelt-job-fold", pid))
                            .size(px(14.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(job.children, |s| {
                                s.child(theme::chevron(unit(open), MUTED, 11.)).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        if !this.toolbelt.collapsed.remove(&pid) {
                                            this.toolbelt.collapsed.insert(pid);
                                        }
                                        cx.notify();
                                    }),
                                )
                            }),
                    )
                    .when_some(job.agent, |s, _| {
                        s.child(theme::icon(theme::ui("bot"), MODIFIED, 13.))
                    })
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_family(theme::mono_font())
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(pid.to_string()),
                    )
            })
            .collect::<Vec<_>>();
        let list = div()
            .id("toolbelt-jobs")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .pb_1()
            .overflow_y_scroll()
            .track_scroll(&self.toolbelt.jobs_scroll)
            .children(rows)
            .when_some(empty, |s, text| {
                s.child(div().px_3().py_2().text_color(rgb(MUTED)).child(text))
            });
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(list)
            .child(self.jobs_footer(count, cx))
            .into_any_element()
    }

    fn jobs_footer(&mut self, count: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.toolbelt.selected;
        let signal = self.toolbelt.signal;
        let signals = !cfg!(windows);
        let end_tooltip: SharedString = match selected {
            None => "Select a process to end it".into(),
            Some(pid) if signals => format!("Send SIG{} to process {pid}", signal.name()).into(),
            Some(pid) => format!("End process {pid}").into(),
        };
        let menu = self.toolbelt.signal_menu.then(|| {
            div()
                .id("toolbelt-signals")
                .absolute()
                .occlude()
                .bottom(px(34.))
                .left_2()
                .w(px(120.))
                .p_1()
                .rounded_md()
                .bg(rgb(0x121212))
                .border_1()
                .border_color(rgb(0x333333))
                .shadow_xl()
                .flex()
                .flex_col()
                .children(Signal::ALL.map(|option| {
                    theme::chip(
                        ("toolbelt-signal", option as usize),
                        format!("SIG{}", option.name()),
                    )
                    .when(option == signal, |s| {
                        s.text_color(rgb(TEXT)).bg(rgb(SELECTED))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toolbelt.signal = option;
                        this.toolbelt.signal_menu = false;
                        cx.notify();
                    }))
                }))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.toolbelt.signal_menu = false;
                    cx.notify();
                }))
                .with_animation(
                    "toolbelt-signals-in",
                    Animation::new(Duration::from_millis(130)).with_easing(ease_out_quint()),
                    |el, t| el.opacity(t),
                )
        });
        div()
            .h(px(32.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .px_1p5()
            .border_t_1()
            .border_color(rgb(DIVIDER))
            .child(
                theme::text_button(
                    "toolbelt-end",
                    Some("x"),
                    if signals { "Kill" } else { "End process" },
                )
                .when(selected.is_none(), |s| s.opacity(0.45).cursor_default())
                .tooltip(move |window, cx| Tooltip::new(end_tooltip.clone()).build(window, cx))
                .on_click(cx.listener(|this, _, _, cx| this.end_selected(cx))),
            )
            .when(signals, |s| {
                s.child(
                    theme::text_button("toolbelt-signal", None, signal.name())
                        .font_family(theme::mono_font())
                        .text_size(px(11.))
                        .child(theme::icon(theme::ui("chevron-down"), TEXT_2, 12.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toolbelt.signal_menu = !this.toolbelt.signal_menu;
                            cx.notify();
                        })),
                )
            })
            .child(div().flex_1())
            .when(count > 0, |s| {
                s.child(
                    div()
                        .pr_1p5()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child(format!(
                            "{count} process{}",
                            if count == 1 { "" } else { "es" }
                        )),
                )
            })
            .children(menu)
    }

    fn end_selected(&mut self, cx: &mut Context<Self>) {
        self.toolbelt.signal_menu = false;
        let Some(pid) = self.toolbelt.selected else {
            return;
        };
        // Only a process still listed under the focused terminal.
        let listed = self
            .slots
            .get(&self.active)
            .and_then(|s| s.terminal.read(cx).shell)
            .is_some_and(|shell| {
                self.scan
                    .jobs(shell, &HashSet::new())
                    .iter()
                    .any(|j| j.pid == pid)
            });
        if listed {
            match crate::platform::end_process(pid, self.toolbelt.signal) {
                Ok(()) => self.toolbelt.selected = None,
                Err(e) => self.notice = format!("Couldn't end process {pid}: {e}"),
            }
        }
        cx.notify();
    }

    fn status_section(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut entries = Vec::new();
        for (index, layout) in self.tabs.iter().enumerate() {
            let leaves = layout.leaves();
            let agents = leaves
                .iter()
                .filter(|id| self.slots.get(id).is_some_and(|s| s.agent.is_some()))
                .count();
            for (position, id) in leaves.iter().enumerate() {
                let Some(slot) = self.slots.get(id) else {
                    continue;
                };
                let Some(status) = slot.agent.clone() else {
                    continue;
                };
                let mut name = status
                    .name
                    .clone()
                    .unwrap_or_else(|| self.group_name(index, cx));
                if agents > 1 && status.name.is_none() {
                    name = format!("{name} #{}", position + 1);
                }
                let turn = slot.turn.as_ref();
                let prompt = turn
                    .filter(|t| t.active)
                    .map(|t| {
                        t.label
                            .split_once(" · ")
                            .map_or(t.label.as_str(), |(_, p)| p)
                            .to_owned()
                    })
                    .unwrap_or_default();
                let reply = turn.map(|t| t.reply.clone()).unwrap_or_default();
                entries.push((*id, status, name, prompt, reply));
            }
        }
        let header = self.section_header(
            "toolbelt-status-header",
            "SESSION STATUS",
            self.toolbelt.status_open,
            Some(entries.len()),
            cx,
        );
        if !self.toolbelt.status_open {
            return header.into_any_element();
        }
        let now = Instant::now();
        let empty = entries.is_empty();
        let rows = entries
            .into_iter()
            .map(|(id, status, name, prompt, reply)| {
                let focused = id == self.active;
                let detail = match status.state {
                    AgentState::Waiting => status.detail.clone().unwrap_or_default(),
                    AgentState::Working => prompt,
                    _ => String::new(),
                };
                let time = status
                    .since
                    .filter(|_| status.state != AgentState::Idle)
                    .map(|since| agents::elapsed(now.saturating_duration_since(since).as_secs()));
                div()
                    .id(("toolbelt-agent", id))
                    .mx_1()
                    .px_2()
                    .py(px(6.))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(focused, |s| s.bg(rgb(SELECTED)))
                    .when(!focused, |s| s.hover(|s| s.bg(rgb(HOVER))))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.focus_pane(id, window, cx);
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .child(state_dot(status.state, 7.))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.5))
                                    .text_color(rgb(TEXT))
                                    .child(name),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(MUTED))
                                    .child(status.kind.name()),
                            )
                            .child(div().flex_1())
                            .when_some(time, |s, time| {
                                s.child(
                                    div()
                                        .flex_shrink_0()
                                        .text_size(px(11.))
                                        .text_color(rgb(MUTED))
                                        .child(time),
                                )
                            }),
                    )
                    .child(
                        div()
                            .pl(px(14.))
                            .flex()
                            .items_center()
                            .gap_1()
                            .min_w_0()
                            .text_size(px(11.5))
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_color(rgb(match status.state {
                                        AgentState::Idle => MUTED,
                                        state => state_color(state),
                                    }))
                                    .child(state_label(status.state)),
                            )
                            .when(!detail.is_empty(), |s| {
                                s.child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(rgb(TEXT_2))
                                        .child(format!("· {detail}")),
                                )
                            }),
                    )
                    .when(!reply.is_empty(), |s| {
                        s.child(
                            div()
                                .pl(px(14.))
                                .text_size(px(11.5))
                                .line_height(px(16.))
                                .text_color(rgb(MUTED))
                                .line_clamp(2)
                                .child(reply.split_whitespace().collect::<Vec<_>>().join(" ")),
                        )
                    })
            })
            .collect::<Vec<_>>();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id("toolbelt-status")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .pb_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.toolbelt.status_scroll)
                    .children(rows)
                    .when(empty, |s| {
                        s.child(
                            div()
                                .px_3()
                                .py_2()
                                .text_color(rgb(MUTED))
                                .child("No Claude Code or Codex sessions"),
                        )
                    }),
            )
            .into_any_element()
    }
}
