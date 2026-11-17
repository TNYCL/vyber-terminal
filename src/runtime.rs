//! Application-wide monitoring. Moving a terminal never starts a second log watcher.
use crate::{
    agents::Scan,
    notifications::Notifications,
    tasks::{Monitor, TaskReview},
    terminal::Terminal,
};
use gpui::*;
#[cfg(not(test))]
use std::time::Duration;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
#[cfg_attr(test, allow(dead_code))]
pub(crate) enum RuntimeEvent {
    Review(Box<TaskReview>, Option<usize>),
    Notification(usize),
    Scan(Scan),
}

pub(crate) struct Runtime {
    terminals: HashMap<WindowId, HashMap<usize, WeakEntity<Terminal>>>,
    roots: Arc<Mutex<Vec<PathBuf>>>,
    #[cfg_attr(test, allow(dead_code))]
    monitor: Monitor,
    notifications: Notifications,
    sessions: HashMap<String, usize>,
    next: usize,
    pub(crate) scan: Scan,
}
impl EventEmitter<RuntimeEvent> for Runtime {}
struct SharedRuntime(Entity<Runtime>);
impl Global for SharedRuntime {}

pub(crate) fn entity(cx: &App) -> Entity<Runtime> {
    cx.global::<SharedRuntime>().0.clone()
}

pub(crate) fn init(cx: &mut App) {
    let runtime = cx.new(|_| {
        let roots = Arc::new(Mutex::new(Vec::new()));
        #[cfg(not(test))]
        let monitor = Monitor::start(roots.clone());
        #[cfg(test)]
        let monitor = Monitor::inactive();
        Runtime {
            terminals: HashMap::new(),
            roots,
            monitor,
            notifications: Notifications::new(),
            sessions: HashMap::new(),
            next: 0,
            scan: Scan::default(),
        }
    });
    cx.set_global(SharedRuntime(runtime.clone()));
    #[cfg(not(test))]
    cx.spawn(async move |cx| {
        let runtime = runtime.downgrade();
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(350))
                .await;
            if runtime.update(cx, |runtime, cx| runtime.poll(cx)).is_err() {
                break;
            }
        }
    })
    .detach();
    #[cfg(not(test))]
    {
        let runtime = entity(cx).downgrade();
        cx.spawn(async move |cx| {
            loop {
                let Ok(shells) = runtime.update(cx, |r, cx| {
                    r.live(cx)
                        .iter()
                        .filter_map(|(_, t)| t.read(cx).shell)
                        .collect::<Vec<_>>()
                }) else {
                    break;
                };
                let scan = cx
                    .background_executor()
                    .spawn(async move { Scan::capture(&shells) })
                    .await;
                if runtime
                    .update(cx, |runtime, cx| {
                        runtime.scan = scan.clone();
                        cx.emit(RuntimeEvent::Scan(scan));
                    })
                    .is_err()
                {
                    break;
                }
                cx.background_executor().timer(Duration::from_secs(1)).await;
            }
        })
        .detach();
    }
}

impl Runtime {
    pub(crate) fn allocate(&mut self, preferred: Option<usize>, cx: &App) -> usize {
        let id = preferred
            .filter(|id| !self.live(cx).iter().any(|(other, _)| id == other))
            .unwrap_or(self.next);
        self.next = self.next.max(id + 1);
        id
    }

    pub(crate) fn sync(
        &mut self,
        window: WindowId,
        terminals: HashMap<usize, WeakEntity<Terminal>>,
        cx: &App,
    ) {
        self.terminals.insert(window, terminals);
        self.refresh_roots(cx);
    }
    pub(crate) fn remove_window(&mut self, window: WindowId, cx: &App) {
        self.terminals.remove(&window);
        self.refresh_roots(cx);
    }
    fn refresh_roots(&mut self, cx: &App) {
        let live = self.live(cx);
        let mut roots: Vec<_> = live.iter().map(|(_, t)| t.read(cx).root.clone()).collect();
        roots.sort();
        roots.dedup();
        *self.roots.lock().unwrap() = roots;
        self.sessions
            .retain(|_, id| live.iter().any(|(live_id, _)| live_id == id));
    }
    fn live(&self, _: &App) -> Vec<(usize, Entity<Terminal>)> {
        let mut live: Vec<_> = self
            .terminals
            .values()
            .flat_map(|slots| slots.iter())
            .filter_map(|(id, t)| t.upgrade().map(|t| (*id, t)))
            .collect();
        live.sort_by_key(|(id, _)| *id);
        live.dedup_by_key(|(id, _)| *id);
        live
    }
    pub(crate) fn notify(&mut self, id: usize, message: String) {
        self.notifications.show(id, message);
    }
    #[cfg(not(test))]
    fn poll(&mut self, cx: &mut Context<Self>) {
        for review in self.monitor.receiver.try_iter().collect::<Vec<_>>() {
            let owner = self.turn_terminal(&review, cx);
            cx.emit(RuntimeEvent::Review(Box::new(review), owner));
        }
        for id in self.notifications.receiver.try_iter().collect::<Vec<_>>() {
            cx.emit(RuntimeEvent::Notification(id));
        }
    }
    #[cfg(not(test))]
    fn turn_terminal(&mut self, review: &TaskReview, cx: &App) -> Option<usize> {
        if review.session.is_empty() || !review.is_from_terminal() {
            return None;
        }
        let live = self.live(cx);
        if let Some(id) = self.sessions.get(&review.session)
            && live.iter().any(|(other, _)| other == id)
        {
            return Some(*id);
        }
        let agent = (review.agent == "Claude")
            .then(|| crate::tasks::claude_process(&review.session))
            .flatten();
        if agent.is_none() && (!review.active || review.after.is_some()) {
            return None;
        }
        let processes = crate::platform::Processes::list();
        let id = if let Some(agent) = agent {
            live.iter()
                .find(|(_, t)| {
                    t.read(cx)
                        .shell
                        .is_some_and(|shell| processes.descends(agent, shell))
                })?
                .0
        } else {
            let program = review.agent.to_lowercase();
            let normalize = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
            let prompt = normalize(review.label.split_once(" · ").map_or("", |(_, p)| p));
            let now = std::time::Instant::now();
            live.iter()
                .filter_map(|(id, t)| {
                    let t = t.read(cx);
                    let submitted = t.last_submit?;
                    if now.duration_since(submitted) > Duration::from_secs(45)
                        || !crate::tasks::matches_root(&review.root, &t.root)
                    {
                        return None;
                    }
                    let runs = t.shell.is_some_and(|shell| processes.runs(shell, &program));
                    let typed = normalize(&t.last_line);
                    let matched = typed.len() >= 3
                        && prompt.len() >= 3
                        && (prompt.starts_with(&typed) || typed.starts_with(&prompt));
                    (runs || matched).then_some((matched, runs, submitted, *id))
                })
                .max_by_key(|(matched, runs, submitted, _)| (*matched, *runs, *submitted))?
                .3
        };
        self.sessions.insert(review.session.clone(), id);
        Some(id)
    }
}
