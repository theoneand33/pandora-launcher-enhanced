use std::{cell::RefCell, num::NonZeroUsize, ops::Range, rc::Rc, sync::Arc};

use crate::{
    component::scrollable_text::{
        ScrollHandler, ScrollRenderInfo, ScrollState, ScrollableLine, Scrolling, WrappedLineCache, WrappedLines,
        update_scrolling,
    },
    fenwick::FenwickTree,
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme as _, Icon, Sizable,
    button::Button,
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::{Scrollbar, ScrollbarHandle},
    v_flex,
};
use lru::LruCache;
use rustc_hash::FxBuildHasher;

use bridge::{game_output::GameOutputLogLevel, message::GameOutputMsg};

use crate::{CloseWindow, icon::PandoraIcon};

struct CachedShapedLogLevels {
    fatal: Arc<ShapedLine>,
    error: Arc<ShapedLine>,
    warn: Arc<ShapedLine>,
    info: Arc<ShapedLine>,
    debug: Arc<ShapedLine>,
    trace: Arc<ShapedLine>,
    other: Arc<ShapedLine>,
}

struct CachedShapedLines {
    last_time: Option<Arc<ShapedLine>>,
    last_time_millis: i64,

    item_lines: LruCache<usize, WrappedLines, FxBuildHasher>,
}

impl WrappedLineCache for CachedShapedLines {
    fn get_lines(&mut self, index: usize) -> Option<&WrappedLines> {
        self.item_lines.get(&index)
    }

    fn put_lines(&mut self, index: usize, lines: WrappedLines) {
        self.item_lines.put(index, lines);
    }
}

pub struct GameOutputItemState {
    items: Vec<GameOutputItem>,
    last_scrolled_item: usize,
    item_sizes: FenwickTree,
    total_line_count: usize,
    cached_shaped_lines: CachedShapedLines,
    search_query: SharedString,
}

pub struct GameOutput {
    font: Font,
    scroll_state: Rc<RefCell<ScrollState>>,
    pending: Vec<GameOutputMsg>,
    item_state: Option<GameOutputItemState>,
    time_column_width: Pixels,
    level_column_width: Pixels,
    shaped_log_levels: Option<CachedShapedLogLevels>,
    _receive_lines_task: Task<()>,
}

impl GameOutput {
    pub fn new(mut receiver: tokio::sync::mpsc::UnboundedReceiver<GameOutputMsg>, cx: &mut Context<Self>) -> Self {
        let task = cx.spawn(async move |output, cx| {
            loop {
                let Some(msg) = receiver.recv().await else {
                    return;
                };
                _ = output.update(cx, |output, cx| {
                    output.pending.push(msg);

                    while let Ok(msg) = receiver.try_recv() {
                        output.pending.push(msg);
                    }

                    cx.notify();
                });
            }
        });

        Self {
            font: Font {
                family: SharedString::new_static("Roboto Mono"),
                features: FontFeatures::default(),
                fallbacks: None,
                weight: FontWeight::NORMAL,
                style: FontStyle::Normal,
            },
            scroll_state: Default::default(),
            pending: Default::default(),
            item_state: Some(GameOutputItemState {
                items: Vec::new(),
                last_scrolled_item: 0,
                item_sizes: FenwickTree::new(),
                total_line_count: 0,
                cached_shaped_lines: CachedShapedLines {
                    last_time: None,
                    last_time_millis: 0,
                    item_lines: LruCache::with_hasher(NonZeroUsize::new(256).unwrap(), FxBuildHasher),
                },
                search_query: SharedString::new_static(""),
            }),
            time_column_width: Default::default(),
            level_column_width: Default::default(),
            shaped_log_levels: None,
            _receive_lines_task: task,
        }
    }

    fn shape_log_level(
        &self,
        level: &'static str,
        color: Hsla,
        text_system: &Arc<WindowTextSystem>,
        text_style: &TextStyle,
        font_size: Pixels,
    ) -> Arc<ShapedLine> {
        let level_run = TextRun {
            len: level.len(),
            font: self.font.clone(),
            color,
            background_color: text_style.background_color,
            underline: text_style.underline,
            strikethrough: text_style.strikethrough,
        };
        Arc::new(text_system.shape_line(SharedString::new_static(level), font_size, &[level_run], None))
    }

    pub fn apply_pending(&mut self, window: &mut Window, _cx: &mut App) {
        if self.shaped_log_levels.is_none() {
            let text_style = window.text_style();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let text_system = window.text_system();

            let levels = CachedShapedLogLevels {
                fatal: self.shape_log_level("FATAL", hsla(0.0, 0.737, 0.418, 1.0), text_system, &text_style, font_size), // red-700
                error: self.shape_log_level("ERROR", hsla(0.0, 0.842, 0.602, 1.0), text_system, &text_style, font_size), // red-500
                warn: self.shape_log_level(
                    "WARN",
                    hsla(24.6 / 360.0, 0.95, 0.531, 1.0),
                    text_system,
                    &text_style,
                    font_size,
                ), // orange-500
                info: self.shape_log_level(
                    "INFO",
                    hsla(83.7 / 360.0, 0.805, 0.443, 1.0),
                    text_system,
                    &text_style,
                    font_size,
                ), // lime-500
                debug: self.shape_log_level(
                    "DEBUG",
                    hsla(258.3 / 360.0, 0.895, 0.663, 1.0),
                    text_system,
                    &text_style,
                    font_size,
                ), // violet-500
                trace: self.shape_log_level(
                    "TRACE",
                    hsla(198.6 / 360.0, 0.887, 0.484, 1.0),
                    text_system,
                    &text_style,
                    font_size,
                ), // sky-500
                other: self.shape_log_level("OTHER", hsla(0.0, 0.5, 0.5, 1.0), text_system, &text_style, font_size),
            };

            self.level_column_width = levels
                .fatal
                .width
                .max(levels.error.width)
                .max(levels.warn.width)
                .max(levels.info.width)
                .max(levels.debug.width)
                .max(levels.trace.width)
                .max(levels.other.width)
                + font_size / 2.0;
            self.shaped_log_levels = Some(levels);
        }
        let Some(item_state) = &mut self.item_state else {
            return;
        };
        for msg in self.pending.drain(..) {
            let shaped_level = match msg.level {
                GameOutputLogLevel::Fatal => self.shaped_log_levels.as_ref().unwrap().fatal.clone(),
                GameOutputLogLevel::Error => self.shaped_log_levels.as_ref().unwrap().error.clone(),
                GameOutputLogLevel::Warn => self.shaped_log_levels.as_ref().unwrap().warn.clone(),
                GameOutputLogLevel::Info => self.shaped_log_levels.as_ref().unwrap().info.clone(),
                GameOutputLogLevel::Debug => self.shaped_log_levels.as_ref().unwrap().debug.clone(),
                GameOutputLogLevel::Trace => self.shaped_log_levels.as_ref().unwrap().trace.clone(),
                GameOutputLogLevel::Other => self.shaped_log_levels.as_ref().unwrap().other.clone(),
            };

            let mut highlighted_text = None;

            if !item_state.search_query.is_empty() {
                for (line_index, line) in msg.text.iter().enumerate() {
                    if let Some(found) = line.find(item_state.search_query.as_str()) {
                        highlighted_text = Some((line_index, found..found + item_state.search_query.as_str().len()));
                        break;
                    }
                }
                if highlighted_text.is_none() {
                    // Item doesn't match search query, push skipped item
                    let backup_total_lines_while_skipped = msg.text.len();
                    item_state.item_sizes.push(0);
                    item_state.items.push(GameOutputItem {
                        time: TimeShapedLine::Timestamp(msg.time),
                        level: shaped_level.clone(),
                        text: msg.text.clone(),
                        index: item_state.items.len(),
                        backup_total_lines_while_skipped,
                        total_lines: 0,
                        highlighted_text: None,
                        skip: true,
                    });
                    continue;
                }
            }

            let total_lines = msg.text.len();
            item_state.item_sizes.push(total_lines);
            item_state.total_line_count += total_lines;
            item_state.items.push(GameOutputItem {
                time: TimeShapedLine::Timestamp(msg.time),
                level: shaped_level.clone(),
                text: msg.text.clone(),
                index: item_state.items.len(),
                backup_total_lines_while_skipped: total_lines,
                total_lines,
                highlighted_text,
                skip: false,
            });
        }
    }
}

pub struct GameOutputList {
    interactivity: Interactivity,
    game_output: Entity<GameOutput>,
}

enum TimeShapedLine {
    Timestamp(i64),
    Shaped(Arc<ShapedLine>),
}

struct GameOutputItem {
    time: TimeShapedLine,
    level: Arc<ShapedLine>,

    text: Arc<[Arc<str>]>,
    index: usize,
    backup_total_lines_while_skipped: usize,
    total_lines: usize,
    highlighted_text: Option<(usize, Range<usize>)>,
    skip: bool,
}

impl ScrollableLine for GameOutputItem {
    fn is_skip(&self) -> bool {
        self.skip
    }

    fn index(&self) -> usize {
        self.index
    }

    fn total_lines(&self) -> usize {
        self.total_lines
    }

    fn set_total_lines(&mut self, lines: usize) {
        self.total_lines = lines;
    }

    fn compute_wrapped_text<'a>(
        &mut self,
        wrap_width: Pixels,
        text_system: &Arc<WindowTextSystem>,
        font: &Font,
        font_size: Pixels,
        text_style: &TextStyle,
        line_wrapper: &mut LineWrapperHandle,
        cache: &'a mut dyn WrappedLineCache,
    ) -> &'a [ShapedLine] {
        let mut recompute = true;

        if let Some(last_wrapped) = cache.get_lines(self.index)
            && (last_wrapped.wrap_width == wrap_width
                || (last_wrapped.lines.len() == 1 && last_wrapped.lines.first().unwrap().width < wrap_width))
        {
            recompute = false;
        }

        if recompute {
            let mut wrapped = Vec::new();
            for (original_line_index, line) in self.text.iter().enumerate() {
                let fragments = [LineFragment::Text { text: line }];
                let boundaries = line_wrapper.wrap_line(&fragments, wrap_width);

                let mut handle_segment = |wrapped_line: SharedString, from, to| {
                    let runs: &[TextRun] = if let Some((highlight_line, highlight_range)) = &self.highlighted_text
                        && *highlight_line == original_line_index
                        && highlight_range.start < to
                        && highlight_range.end > from
                    {
                        let highlight_start = highlight_range.start.max(from);
                        let highlight_end = highlight_range.end.min(to);

                        &[
                            TextRun {
                                len: highlight_start - from,
                                font: font.clone(),
                                color: text_style.color,
                                background_color: text_style.background_color,
                                underline: text_style.underline,
                                strikethrough: text_style.strikethrough,
                            },
                            TextRun {
                                len: highlight_end - highlight_start,
                                font: font.clone(),
                                color: gpui::black(),
                                background_color: Some(gpui::yellow()),
                                underline: text_style.underline,
                                strikethrough: text_style.strikethrough,
                            },
                            TextRun {
                                len: to - highlight_end,
                                font: font.clone(),
                                color: text_style.color,
                                background_color: text_style.background_color,
                                underline: text_style.underline,
                                strikethrough: text_style.strikethrough,
                            },
                        ]
                    } else {
                        &[TextRun {
                            len: wrapped_line.len(),
                            font: font.clone(),
                            color: text_style.color,
                            background_color: text_style.background_color,
                            underline: text_style.underline,
                            strikethrough: text_style.strikethrough,
                        }]
                    };

                    let shaped = text_system.shape_line(wrapped_line, font_size, runs, None);
                    wrapped.push(shaped);
                };

                let mut last_boundary_ix = 0;
                for boundary in boundaries {
                    let wrapped_line = &line[last_boundary_ix..boundary.ix];
                    let wrapped_line = SharedString::new(wrapped_line);
                    (handle_segment)(wrapped_line, last_boundary_ix, boundary.ix);
                    last_boundary_ix = boundary.ix;
                }

                // Push last segment
                let wrapped_line = if last_boundary_ix == 0 {
                    line.into()
                } else {
                    SharedString::new(&line[last_boundary_ix..])
                };
                (handle_segment)(wrapped_line, last_boundary_ix, line.len());
            }

            cache.put_lines(
                self.index,
                WrappedLines {
                    wrap_width,
                    lines: wrapped,
                },
            );
        }

        cache.get_lines(self.index).unwrap().lines.as_slice()
    }
}

impl InteractiveElement for GameOutputList {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl IntoElement for GameOutputList {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GameOutputList {
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
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout_id =
            self.interactivity
                .request_layout(global_id, inspector_id, window, cx, |mut style, window, cx| {
                    style.size.width = relative(1.0).into();
                    style.size.height = relative(1.0).into();
                    window.request_layout(style, None, cx)
                });
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.interactivity
            .prepaint(global_id, inspector_id, bounds, bounds.size, window, cx, |_, _, _, _, _| {})
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            self.interactivity
                .paint(global_id, inspector_id, bounds, None, window, cx, |_, window, cx| {
                    let visible_bounds = bounds;
                    let mut bounds = bounds.inset(px(12.0));
                    bounds.size.width += px(12.0);

                    cx.update_entity(&self.game_output, |game_output, cx| {
                        game_output.apply_pending(window, cx);

                        let text_style = window.text_style();

                        let font_size = text_style.font_size.to_pixels(window.rem_size());
                        let line_height = font_size * 1.25;

                        let text_width =
                            bounds.size.width - game_output.time_column_width - game_output.level_column_width;
                        let wrap_width = text_width.max(font_size * 30);

                        let mut line_wrapper = window.text_system().line_wrapper(game_output.font.clone(), font_size);

                        let scroll_render_info = game_output.update_scrolling(
                            line_height,
                            wrap_width,
                            font_size,
                            &text_style,
                            &mut line_wrapper,
                            window.text_system(),
                        );

                        if let Some(item_state) = game_output.item_state.as_mut()
                            && !item_state.items.is_empty()
                        {
                            if scroll_render_info.reverse {
                                paint_lines::<true>(
                                    item_state.items[..scroll_render_info.item + 1].iter_mut().rev(),
                                    visible_bounds,
                                    bounds,
                                    scroll_render_info.offset,
                                    &game_output.font,
                                    &text_style,
                                    wrap_width,
                                    font_size,
                                    line_height,
                                    &mut game_output.time_column_width,
                                    game_output.level_column_width,
                                    &mut item_state.item_sizes,
                                    &mut item_state.total_line_count,
                                    &mut line_wrapper,
                                    &mut item_state.cached_shaped_lines,
                                    window,
                                    cx,
                                );
                            } else {
                                paint_lines::<false>(
                                    item_state.items[scroll_render_info.item..].iter_mut(),
                                    visible_bounds,
                                    bounds,
                                    scroll_render_info.offset,
                                    &game_output.font,
                                    &text_style,
                                    wrap_width,
                                    font_size,
                                    line_height,
                                    &mut game_output.time_column_width,
                                    game_output.level_column_width,
                                    &mut item_state.item_sizes,
                                    &mut item_state.total_line_count,
                                    &mut line_wrapper,
                                    &mut item_state.cached_shaped_lines,
                                    window,
                                    cx,
                                );
                            }
                        }

                        let mut scroll_state = game_output.scroll_state.borrow_mut();
                        scroll_state.bounds_y = bounds.size.height;
                        scroll_state.line_height = line_height;
                        scroll_state.lines = if let Some(item_state) = &game_output.item_state {
                            item_state.total_line_count
                        } else {
                            0
                        };
                    });
                });
        });
    }
}

impl GameOutput {
    fn update_scrolling(
        &mut self,
        line_height: Pixels,
        wrap_width: Pixels,
        font_size: Pixels,
        text_style: &TextStyle,
        line_wrapper: &mut LineWrapperHandle,
        text_system: &Arc<WindowTextSystem>,
    ) -> ScrollRenderInfo {
        let Some(item_state) = self.item_state.as_mut() else {
            self.scroll_state.borrow_mut().scrolling = Scrolling::Bottom;
            return ScrollRenderInfo {
                item: 0,
                reverse: true,
                offset: Pixels::ZERO,
            };
        };

        let mut scroll_state = self.scroll_state.borrow_mut();
        update_scrolling(
            &mut scroll_state,
            &self.font,
            &mut item_state.items,
            &mut item_state.last_scrolled_item,
            &mut item_state.item_sizes,
            &mut item_state.total_line_count,
            &mut item_state.cached_shaped_lines,
            line_height,
            wrap_width,
            font_size,
            text_style,
            line_wrapper,
            text_system,
        )
    }
}

fn paint_lines<'a, const REVERSE: bool>(
    items: impl Iterator<Item = &'a mut GameOutputItem>,
    visible_bounds: Bounds<Pixels>,
    bounds: Bounds<Pixels>,
    offset: Pixels,
    font: &Font,
    text_style: &TextStyle,
    wrap_width: Pixels,
    font_size: Pixels,
    line_height: Pixels,
    time_column_width: &mut Pixels,
    level_column_width: Pixels,
    item_sizes: &mut FenwickTree,
    total_line_count: &mut usize,
    line_wrapper: &mut LineWrapperHandle,
    cache: &mut CachedShapedLines,
    window: &mut Window,
    cx: &mut App,
) {
    let mut text_origin = bounds.origin;
    if REVERSE {
        text_origin.y += bounds.size.height;
        text_origin.y -= line_height;
    }
    text_origin.y += offset;

    for item in items {
        if item.is_skip() {
            continue;
        }
        let has_highlighted_text = item.highlighted_text.is_some();

        let lines = item.compute_wrapped_text(
            wrap_width,
            window.text_system(),
            font,
            font_size,
            text_style,
            line_wrapper,
            cache,
        );

        let line_count = lines.len().max(1);

        let mut line_origin = text_origin;
        line_origin.x += *time_column_width + level_column_width;
        if REVERSE {
            for shaped in lines.iter().rev() {
                if line_origin.y <= visible_bounds.origin.y + visible_bounds.size.height {
                    if has_highlighted_text {
                        _ = shaped.paint_background(line_origin, line_height, TextAlign::Left, None, window, cx);
                    }
                    _ = shaped.paint(line_origin, line_height, TextAlign::Left, None, window, cx);
                }
                line_origin.y -= line_height;
            }
        } else {
            for shaped in lines.iter() {
                if line_origin.y >= visible_bounds.origin.y - line_height {
                    if has_highlighted_text {
                        _ = shaped.paint_background(line_origin, line_height, TextAlign::Left, None, window, cx);
                    }
                    _ = shaped.paint(line_origin, line_height, TextAlign::Left, None, window, cx);
                }
                line_origin.y += line_height;
            }
        }

        // Shape time text if needed
        if let TimeShapedLine::Timestamp(timestamp) = item.time {
            if let Some(last_shaped_time) = &cache.last_time
                && cache.last_time_millis == timestamp
            {
                item.time = TimeShapedLine::Shaped(Arc::clone(last_shaped_time));
            } else {
                let date_time = chrono::DateTime::from_timestamp_millis(timestamp)
                    .map(|dt| dt.with_timezone(&chrono::Local))
                    .unwrap_or_else(chrono::Local::now);
                let time = format!("{}", date_time.time().format("%H:%M:%S%.3f"));
                let time_run = TextRun {
                    len: time.len(),
                    font: font.clone(),
                    color: text_style.color,
                    background_color: text_style.background_color,
                    underline: text_style.underline,
                    strikethrough: text_style.strikethrough,
                };
                let shaped_time = Arc::new(window.text_system().shape_line(time.into(), font_size, &[time_run], None));

                item.time = TimeShapedLine::Shaped(Arc::clone(&shaped_time));

                *time_column_width = (*time_column_width).max(shaped_time.width + font_size / 2.0);

                cache.last_time = Some(shaped_time);
                cache.last_time_millis = timestamp;
            }
        }

        // Render time text
        let mut time_origin = text_origin;
        if REVERSE {
            time_origin.y -= (line_count - 1) * line_height;
        }
        if let TimeShapedLine::Shaped(shaped_time) = &item.time {
            _ = shaped_time.paint(time_origin, line_height, TextAlign::Left, None, window, cx);
        }

        let mut level_origin = time_origin;
        level_origin.x += *time_column_width + level_column_width - item.level.width - font_size / 2.0;
        _ = item.level.paint(level_origin, line_height, TextAlign::Left, None, window, cx);

        if line_count != item.total_lines() {
            if item.total_lines() < line_count {
                item_sizes.add_at(item.index(), line_count - item.total_lines());
                *total_line_count += line_count - item.total_lines();
            } else {
                item_sizes.sub_at(item.index(), item.total_lines() - line_count);
                *total_line_count -= item.total_lines() - line_count;
            }
            item.set_total_lines(line_count);
        }

        if REVERSE {
            text_origin.y -= line_count * line_height;
            if text_origin.y < visible_bounds.origin.y - line_height {
                break;
            }
        } else {
            text_origin.y += line_count * line_height;
            if text_origin.y > visible_bounds.origin.y + visible_bounds.size.height {
                break;
            }
        }
    }
}

pub struct GameOutputRoot {
    scroll_handler: ScrollHandler,
    game_output: Entity<GameOutput>,
    search_state: Entity<InputState>,
    _search_task: Task<()>,
    _search_input_subscription: Subscription,
    focus_handle: FocusHandle,
}

impl GameOutputRoot {
    pub fn new(game_output: Entity<GameOutput>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let scroll_state = Rc::clone(&game_output.read(cx).scroll_state);

        let search_state = cx.new(|cx| InputState::new(window, cx).placeholder(t::common::search()).clean_on_escape());

        let _search_input_subscription = cx.subscribe_in(&search_state, window, Self::on_search_input_event);

        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        Self {
            scroll_handler: ScrollHandler { state: scroll_state },
            game_output,
            search_state,
            _search_task: Task::ready(()),
            _search_input_subscription,
            focus_handle,
        }
    }

    fn on_search_input_event(
        &mut self,
        state: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let InputEvent::PressEnter {
            secondary: false,
            shift: _,
        } = event
        else {
            return;
        };

        let item_state = self.game_output.update(cx, |game_output, _| game_output.item_state.take());

        let Some(mut item_state) = item_state else {
            return; // Already searching
        };

        let search_pattern = state.read(cx).value();
        if search_pattern.trim().is_empty() {
            self._search_task = cx.spawn_in(window, async move |this, window| {
                let mut lengths = Vec::new();
                item_state.total_line_count = 0;
                for item in &mut item_state.items {
                    if item.skip {
                        item.total_lines = item.backup_total_lines_while_skipped;
                    }

                    item.skip = false;
                    item.highlighted_text = None;

                    item_state.total_line_count += item.total_lines;
                    lengths.push(item.total_lines);
                }
                item_state.item_sizes = FenwickTree::from_iter(lengths.into_iter());
                item_state.cached_shaped_lines.item_lines.clear();
                item_state.search_query = SharedString::new_static("");

                this.update_in(window, |this, window, cx| {
                    this.game_output.update(cx, |game_output, _| {
                        game_output.item_state = Some(item_state);
                    });
                    this.search_state.update(cx, |input, cx| input.set_loading(false, window, cx));
                    cx.notify();
                })
                .unwrap();
            });
        } else {
            self._search_task = cx.spawn_in(window, async move |this, window| {
                let mut lengths = Vec::new();
                item_state.total_line_count = 0;
                for item in &mut item_state.items {
                    let mut contains = None;
                    for (line_index, line) in item.text.iter().enumerate() {
                        if let Some(found) = line.find(search_pattern.as_str()) {
                            contains = Some((line_index, found..found + search_pattern.as_str().len()));
                            break;
                        }
                    }
                    if contains.is_some() {
                        lengths.push(item.total_lines);
                        item_state.total_line_count += item.total_lines;

                        item.highlighted_text = contains;
                        item.skip = false;
                    } else {
                        item.backup_total_lines_while_skipped = item.total_lines;
                        item.total_lines = 0;
                        lengths.push(0);

                        item.skip = true;
                    }
                }
                item_state.item_sizes = FenwickTree::from_iter(lengths.into_iter());
                item_state.cached_shaped_lines.item_lines.clear();
                item_state.search_query = search_pattern;

                this.update_in(window, |this, window, cx| {
                    this.game_output.update(cx, |game_output, _| {
                        game_output.item_state = Some(item_state);
                    });
                    this.search_state.update(cx, |input, cx| input.set_loading(false, window, cx));
                    cx.notify();
                })
                .unwrap();
            });
        }

        state.update(cx, |input, cx| input.set_loading(true, window, cx));
    }
}

impl Render for GameOutputRoot {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let search = Input::new(&self.search_state).prefix(Icon::new(PandoraIcon::Search).small());

        let bar =
            h_flex()
                .w_full()
                .rounded(cx.theme().radius)
                .id("controls")
                .flex_1()
                .gap_4()
                .child(search)
                .child(Button::new("top").label(t::common::nav::top()).on_click(cx.listener(|root, _, _, cx| {
                    let mut state = root.scroll_handler.state.borrow_mut();
                    state.scrolling = Scrolling::Top { offset: Pixels::ZERO };
                    cx.notify();
                })))
                .child(Button::new("bottom").label(t::common::nav::bottom()).on_click(cx.listener(
                    |root, _, _, cx| {
                        let mut state = root.scroll_handler.state.borrow_mut();
                        state.scrolling = Scrolling::Bottom;
                        cx.notify();
                    },
                )));

        v_flex()
            .size_full()
            .border_12()
            .gap_4()
            .child(bar)
            .child(
                h_flex()
                    .size_full()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(GameOutputList {
                        interactivity: Interactivity::new(),
                        game_output: self.game_output.clone(),
                    })
                    .child(div().w_3().h_full().border_y_12().child(Scrollbar::vertical(&self.scroll_handler))),
            )
            .on_scroll_wheel(cx.listener(|root, event: &ScrollWheelEvent, _, cx| {
                let state = root.scroll_handler.state.borrow();
                let delta = event.delta.pixel_delta(state.line_height).y;
                let max_scroll_amount = state.max_scroll_amount();
                drop(state);

                let current_offset = root.scroll_handler.offset().y;
                let new_offset = (current_offset + delta).clamp(-max_scroll_amount, Pixels::ZERO);
                if current_offset != new_offset {
                    root.scroll_handler.set_offset(Point::new(Pixels::ZERO, new_offset));
                    cx.notify();
                }
            }))
            .track_focus(&self.focus_handle)
            .on_action(|_: &CloseWindow, window, _| {
                window.remove_window();
            })
    }
}
