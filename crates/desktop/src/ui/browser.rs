//! The browse view — the port of `browser.ts`.
//!
//! The CLI's three rendering rules (clamp every line to the terminal width,
//! repaint only changed lines, never repaint on a size report) are all gone:
//! they were compensating for a dumb terminal, and they are the framework's
//! problem now. What survives is the part that was actually about gittles —
//! the layout, the key map, the marking model, and the rule that a failed
//! unstar stays marked.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use gittles_core::{GitHub, Star, Store, search};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, ScrollStrategy, SharedString, Styled,
    UniformListScrollHandle, Window, div, px, rgb, uniform_list,
};
use gpui_component::input::{Input, InputEvent, InputState};
use jiff::Timestamp;

use super::format::{group_digits, language_color, relative_time};

const TAGLINE: &str = "like skittles for your GitHub stars";

// The CLI's palette, lifted out of the 256-colour cube.
const BG: u32 = 0x1c1917;
const ROW_SELECTED: u32 = 0x2c2825;
const TEXT: u32 = 0xfafaf9;
const DIM: u32 = 0xa8a29e;
const FAINT: u32 = 0x57534e;
const CYAN: u32 = 0x22d3ee;
const GREEN: u32 = 0x4ade80;
const YELLOW: u32 = 0xfacc15;
const RED: u32 = 0xf87171;

const ROW_HEIGHT: f32 = 26.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    List,
    Search,
    Help,
    /// A network commit is in flight; input is ignored until it lands.
    Busy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Info,
    Good,
    Warn,
    Bad,
}

impl Tone {
    fn color(self) -> u32 {
        match self {
            Tone::Info => DIM,
            Tone::Good => GREEN,
            Tone::Warn => YELLOW,
            Tone::Bad => RED,
        }
    }
}

/// Progress from the unstar worker, which runs on the tokio runtime rather
/// than gpui's executor — reqwest needs a tokio reactor.
enum Commit {
    Progress {
        index: usize,
        total: usize,
        full_name: String,
    },
    Done {
        removed: Vec<u64>,
        failed: usize,
    },
}

pub struct Browser {
    all: Vec<Star>,
    /// Indices into `all`, in display order.
    rows: Vec<usize>,
    selected: usize,
    marked: HashSet<u64>,
    mode: Mode,
    status: Option<(Tone, SharedString)>,
    query: String,
    username: String,
    last_synced_at: String,
    /// Captured once at construction so row rendering stays pure.
    now: Timestamp,

    store: Store,
    runtime: Arc<tokio::runtime::Runtime>,
    search_input: Entity<InputState>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

impl Browser {
    pub fn new(
        store: Store,
        runtime: Arc<tokio::runtime::Runtime>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let config = store.load_config();
        let all = store.load_stars();
        let rows = (0..all.len()).collect();

        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("search name, description, language")
        });

        cx.subscribe_in(
            &search_input,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = input.read(cx).value().to_string();
                    this.set_query(query, cx);
                }
            },
        )
        .detach();

        Browser {
            all,
            rows,
            selected: 0,
            marked: HashSet::new(),
            mode: Mode::List,
            status: None,
            query: String::new(),
            username: config.username,
            last_synced_at: config.last_synced_at,
            now: Timestamp::now(),
            store,
            runtime,
            search_input,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
        }
    }

    fn selected_star(&self) -> Option<&Star> {
        self.rows.get(self.selected).map(|&index| &self.all[index])
    }

    fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        self.query = query;
        self.rows = search::filter(&self.all, &self.query);
        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }

        let last = self.rows.len() as isize - 1;
        let next = (self.selected as isize + delta).clamp(0, last) as usize;
        self.select(next, cx);
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        self.scroll.scroll_to_item(index, ScrollStrategy::Center);
        cx.notify();
    }

    /// One row's worth of list height, used for page up/down.
    fn page(&self) -> isize {
        12
    }

    fn enter_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = Mode::Search;
        let handle = self.search_input.focus_handle(cx);
        window.focus(&handle);
        cx.notify();
    }

    fn leave_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = Mode::List;
        window.focus(&self.focus);
        cx.notify();
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.set_query(String::new(), cx);
    }

    fn toggle_mark(&mut self, cx: &mut Context<Self>) {
        let Some(star) = self.selected_star() else {
            return;
        };

        let id = star.id;
        if !self.marked.remove(&id) {
            self.marked.insert(id);
        }

        // Marking walks down the list, so a run of repos can be marked without
        // moving your hand.
        self.move_selection(1, cx);
        cx.notify();
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        let Some(star) = self.selected_star() else {
            return;
        };

        let (url, full_name) = (star.url.clone(), star.full_name.clone());
        cx.open_url(&url);
        self.status = Some((Tone::Good, format!("opened {full_name}").into()));
        cx.notify();
    }

    /// Unstar everything marked. Only what GitHub actually accepted leaves the
    /// local store — otherwise a failed unstar would vanish from the list while
    /// still being starred on GitHub.
    fn commit_unstars(&mut self, cx: &mut Context<Self>) {
        if self.marked.is_empty() {
            return;
        }

        let token = self.store.load_config().token;
        if token.is_empty() {
            self.status = Some((Tone::Bad, "sign in first: gittles --sync".into()));
            cx.notify();
            return;
        }

        let targets: Vec<(u64, String)> = self
            .all
            .iter()
            .filter(|star| self.marked.contains(&star.id))
            .map(|star| (star.id, star.full_name.clone()))
            .collect();

        let total = targets.len();
        self.mode = Mode::Busy;
        cx.notify();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Commit>();

        self.runtime.spawn(async move {
            let github = match GitHub::new(token) {
                Ok(github) => github,
                Err(_) => {
                    let _ = tx.send(Commit::Done {
                        removed: Vec::new(),
                        failed: total,
                    });
                    return;
                }
            };

            let mut removed = Vec::new();
            let mut failed = 0;

            for (index, (id, full_name)) in targets.into_iter().enumerate() {
                let _ = tx.send(Commit::Progress {
                    index: index + 1,
                    total,
                    full_name: full_name.clone(),
                });

                match github.unstar(&full_name).await {
                    Ok(()) => removed.push(id),
                    Err(_) => failed += 1,
                }
            }

            let _ = tx.send(Commit::Done { removed, failed });
        });

        cx.spawn(async move |this, cx| {
            while let Some(message) = rx.recv().await {
                let finished = matches!(message, Commit::Done { .. });
                if this
                    .update(cx, |this, cx| this.apply_commit(message, cx))
                    .is_err()
                {
                    break;
                }

                if finished {
                    break;
                }
            }
        })
        .detach();
    }

    fn apply_commit(&mut self, message: Commit, cx: &mut Context<Self>) {
        match message {
            Commit::Progress {
                index,
                total,
                full_name,
            } => {
                self.status = Some((
                    Tone::Info,
                    format!("unstarring {full_name} ({index}/{total})…").into(),
                ));
            }
            Commit::Done { removed, failed } => {
                let removed: HashSet<u64> = removed.into_iter().collect();
                let done = removed.len();

                self.all.retain(|star| !removed.contains(&star.id));
                // Anything that failed stays marked, so it can be retried.
                self.marked.retain(|id| !removed.contains(id));

                if let Err(error) = self.store.save_stars(&self.all) {
                    self.status = Some((Tone::Bad, format!("could not save: {error}").into()));
                } else {
                    self.status = Some(if failed == 0 {
                        (Tone::Good, format!("unstarred {done}").into())
                    } else {
                        (
                            Tone::Warn,
                            format!("unstarred {done}, {failed} failed").into(),
                        )
                    });
                }

                self.rows = search::filter(&self.all, &self.query);
                if self.selected >= self.rows.len() {
                    self.selected = self.rows.len().saturating_sub(1);
                }

                self.mode = Mode::List;
            }
        }

        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let shift = event.keystroke.modifiers.shift;

        match self.mode {
            // Ignore input while the network is busy, but keep it from reaching
            // the search field underneath.
            Mode::Busy => cx.stop_propagation(),

            Mode::Help => {
                self.mode = Mode::List;
                cx.stop_propagation();
                cx.notify();
            }

            // Everything not listed here belongs to the text field.
            Mode::Search => match key {
                "escape" | "enter" => {
                    self.leave_search(window, cx);
                    cx.stop_propagation();
                }
                "up" | "down" => {
                    self.leave_search(window, cx);
                    self.move_selection(if key == "up" { -1 } else { 1 }, cx);
                    cx.stop_propagation();
                }
                _ => {}
            },

            Mode::List => {
                self.status = None;
                cx.stop_propagation();

                match key {
                    "up" | "k" => self.move_selection(-1, cx),
                    "down" | "j" => self.move_selection(1, cx),
                    "pageup" => self.move_selection(-self.page(), cx),
                    "pagedown" => self.move_selection(self.page(), cx),
                    "home" => self.select(0, cx),
                    "end" => self.select(self.rows.len().saturating_sub(1), cx),
                    "g" if shift => self.select(self.rows.len().saturating_sub(1), cx),
                    "g" => self.select(0, cx),
                    "/" | "s" => self.enter_search(window, cx),
                    "x" => self.clear_search(window, cx),
                    "?" => {
                        self.mode = Mode::Help;
                        cx.notify();
                    }
                    "u" if shift => {
                        self.marked.clear();
                        self.status = Some((Tone::Info, "cleared marks".into()));
                        cx.notify();
                    }
                    "o" => self.open_selected(cx),
                    "d" => self.toggle_mark(cx),
                    "c" => self.commit_unstars(cx),
                    "q" => cx.quit(),
                    _ => {}
                }
            }
        }
    }
}

// ---------------------------------------------------------------- rendering

fn dim_text(text: impl Into<SharedString>) -> impl IntoElement {
    div().text_color(rgb(DIM)).child(text.into())
}

impl Browser {
    fn header(&self) -> impl IntoElement {
        let account = if self.username.is_empty() {
            "not signed in".to_string()
        } else {
            self.username.clone()
        };

        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(16.))
            .pt(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(CYAN))
                            .child("★ GITTLES"),
                    )
                    .child(
                        div()
                            .text_color(rgb(FAINT))
                            .text_size(px(12.))
                            .child(TAGLINE),
                    )
                    .child(div().text_color(rgb(FAINT)).child("│"))
                    .child(div().text_color(rgb(GREEN)).child(account)),
            )
            .child(div().text_color(rgb(DIM)).text_size(px(12.)).child(format!(
                "{} stars · synced {} · {} shown",
                group_digits(self.all.len() as u64),
                relative_time(&self.last_synced_at, self.now),
                group_digits(self.rows.len() as u64),
            )))
    }

    fn search_row(&self) -> impl IntoElement {
        div()
            .px(px(16.))
            .py(px(10.))
            .child(Input::new(&self.search_input).bordered(true))
    }

    fn row(&self, index: usize) -> AnyElement {
        let star = &self.all[self.rows[index]];
        let is_selected = index == self.selected;
        let is_marked = self.marked.contains(&star.id);

        let name_color = if is_marked {
            RED
        } else if is_selected {
            CYAN
        } else {
            TEXT
        };

        let language = div()
            .w(px(110.))
            .flex()
            .items_center()
            .gap(px(6.))
            .children((!star.language.is_empty()).then(|| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .w(px(8.))
                            .h(px(8.))
                            .rounded_full()
                            .bg(rgb(language_color(&star.language).unwrap_or(FAINT))),
                    )
                    .child(
                        div()
                            .text_color(rgb(DIM))
                            .text_size(px(12.))
                            .child(star.language.clone()),
                    )
            }));

        div()
            .h(px(ROW_HEIGHT))
            // Without this the row sizes to its content, `flex_1` on the name
            // has nothing to distribute, and the trailing columns ragged-edge.
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(16.))
            .when(is_selected, |row| row.bg(rgb(ROW_SELECTED)))
            .child(
                div()
                    .w(px(12.))
                    .text_color(rgb(CYAN))
                    .child(if is_selected { "❯" } else { " " }),
            )
            .child(
                div()
                    .w(px(12.))
                    .text_color(rgb(RED))
                    .child(if is_marked { "✗" } else { " " }),
            )
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .text_color(rgb(name_color))
                    // Marked-for-unstarring reads as struck through, as in the CLI.
                    .when(is_marked, |name| name.line_through())
                    .when(is_selected, |name| name.font_weight(gpui::FontWeight::BOLD))
                    .child(star.full_name.clone()),
            )
            .child(
                div()
                    .w(px(80.))
                    .text_color(rgb(YELLOW))
                    .text_size(px(12.))
                    .child(format!("★ {}", group_digits(star.stargazers_count))),
            )
            .child(
                div()
                    .w(px(72.))
                    .text_color(rgb(DIM))
                    .text_size(px(12.))
                    .child(relative_time(&star.pushed_at, self.now)),
            )
            .child(language)
            .into_any_element()
    }

    fn list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.rows.is_empty() {
            let message = if self.all.is_empty() {
                "no stars stored yet — run `gittles --sync`"
            } else {
                "nothing matches that search"
            };

            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(dim_text(message))
                .into_any_element();
        }

        let entity = cx.entity();

        div()
            .flex_1()
            .child(
                uniform_list(
                    "stars",
                    self.rows.len(),
                    move |range: Range<usize>, _window, cx: &mut App| {
                        let this = entity.read(cx);
                        range.map(|index| this.row(index)).collect::<Vec<_>>()
                    },
                )
                .track_scroll(self.scroll.clone())
                .size_full(),
            )
            .into_any_element()
    }

    /// The selected repo's description. The CLI put this inline under the row
    /// and reserved the line whether or not there was a description, so the
    /// frame never reflowed. `uniform_list` needs rows of equal height, so it
    /// moves here — a fixed-height strip, reserved the same way.
    fn detail(&self) -> impl IntoElement {
        let description = self
            .selected_star()
            .map(|star| star.description.clone())
            .unwrap_or_default();

        div()
            .h(px(30.))
            .px(px(16.))
            .flex()
            .items_center()
            .child(div().text_color(rgb(DIM)).truncate().child(description))
    }

    fn footer(&self) -> impl IntoElement {
        let position = if self.rows.is_empty() {
            "0/0".to_string()
        } else {
            format!("{}/{}", self.selected + 1, self.rows.len())
        };

        let hints = if self.mode == Mode::Search {
            "enter/esc done · ↑↓ move"
        } else {
            "↑↓/jk move · / search · o open · d mark · c commit · ? help · q quit"
        };

        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .px(px(16.))
            .pb(px(12.))
            .pt(px(8.))
            .border_t(px(1.))
            .border_color(rgb(0x292524))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_color(rgb(FAINT))
                            .text_size(px(12.))
                            .child(position),
                    )
                    .child(
                        div()
                            .text_color(rgb(FAINT))
                            .text_size(px(12.))
                            .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
                    )
                    .children((!self.marked.is_empty()).then(|| {
                        div()
                            .text_color(rgb(RED))
                            .text_size(px(12.))
                            .child(format!("{} marked · c to unstar", self.marked.len()))
                    }))
                    .children(self.status.as_ref().map(|(tone, text)| {
                        div()
                            .text_color(rgb(tone.color()))
                            .text_size(px(12.))
                            .child(text.clone())
                    })),
            )
            .child(div().text_color(rgb(FAINT)).text_size(px(12.)).child(hints))
    }

    fn help(&self) -> impl IntoElement {
        let keys = [
            ("↑ ↓ j k", "move"),
            ("pgup pgdn", "page"),
            ("g G", "top / bottom"),
            ("/", "search (esc or enter to leave)"),
            ("x", "clear the search"),
            ("o", "open the selected repo in your browser"),
            ("d", "mark / unmark for unstarring"),
            ("U", "unmark everything"),
            ("c", "commit — unstar everything marked"),
            ("?", "this help"),
            ("q", "quit"),
        ];

        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(24.))
            .child(
                div()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(TEXT))
                    .child("Keys"),
            )
            .children(keys.map(|(key, description)| {
                div()
                    .flex()
                    .gap(px(16.))
                    .child(div().w(px(110.)).text_color(rgb(CYAN)).child(key))
                    .child(dim_text(description))
            }))
            .child(div().pt(px(8.)).child(dim_text("press any key to go back")))
    }
}

impl Focusable for Browser {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Browser {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = if self.mode == Mode::Help {
            self.help().into_any_element()
        } else {
            self.list(cx).into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .text_size(px(14.))
            .track_focus(&self.focus)
            // Capture, not bubble: the search field binds escape itself, and the
            // list keys must not reach it while it has focus.
            .capture_key_down(cx.listener(Self::on_key))
            .child(self.header())
            .child(self.search_row())
            .child(body)
            .child(self.detail())
            .child(self.footer())
    }
}
