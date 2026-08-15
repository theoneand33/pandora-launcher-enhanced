use std::{cell::RefCell, num::NonZeroUsize, rc::Rc, sync::Arc};

use crate::fenwick::FenwickTree;
use gpui::*;
use gpui_component::scroll::ScrollbarHandle;
use lru::LruCache;
use rustc_hash::FxBuildHasher;

pub struct WrappedLines {
    pub wrap_width: Pixels,
    pub lines: Vec<ShapedLine>,
}

pub trait WrappedLineCache {
    fn get_lines(&mut self, index: usize) -> Option<&WrappedLines>;
    fn put_lines(&mut self, index: usize, lines: WrappedLines);
}

pub struct ShapeCache {
    item_lines: LruCache<usize, WrappedLines, FxBuildHasher>,
}

impl ShapeCache {
    pub fn new() -> Self {
        Self {
            item_lines: LruCache::with_hasher(NonZeroUsize::new(256).unwrap(), FxBuildHasher),
        }
    }

    pub fn clear(&mut self) {
        self.item_lines.clear();
    }
}

impl WrappedLineCache for ShapeCache {
    fn get_lines(&mut self, index: usize) -> Option<&WrappedLines> {
        self.item_lines.get(&index)
    }

    fn put_lines(&mut self, index: usize, lines: WrappedLines) {
        self.item_lines.put(index, lines);
    }
}

pub trait ScrollableLine {
    fn is_skip(&self) -> bool;
    fn index(&self) -> usize;
    fn total_lines(&self) -> usize;
    fn set_total_lines(&mut self, lines: usize);

    fn compute_wrapped_text<'a>(
        &mut self,
        wrap_width: Pixels,
        text_system: &Arc<WindowTextSystem>,
        font: &Font,
        font_size: Pixels,
        text_style: &TextStyle,
        line_wrapper: &mut LineWrapperHandle,
        cache: &'a mut dyn WrappedLineCache,
    ) -> &'a [ShapedLine];
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActiveDrag {
    start_content_height: Pixels,
    drag_pivot: Pixels,
    real_pivot: Pixels,
    actual_offset: Pixels,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Scrolling {
    #[default]
    Bottom,
    Top {
        offset: Pixels,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScrollState {
    pub lines: usize,
    pub line_height: Pixels,
    pub bounds_y: Pixels,
    pub scrolling: Scrolling,
    pub active_drag: Option<ActiveDrag>,
}

impl ScrollState {
    fn content_height_for_scrollbar(&self) -> Pixels {
        self.active_drag
            .as_ref()
            .map(|v| v.start_content_height)
            .unwrap_or(self.lines * self.line_height)
    }

    pub fn max_scroll_amount(&self) -> Pixels {
        (self.lines * self.line_height - self.bounds_y).max(Pixels::ZERO)
    }

    pub fn offset(&self) -> Pixels {
        match self.scrolling {
            Scrolling::Bottom => {
                let content_height = self.content_height_for_scrollbar();
                -(content_height - self.bounds_y)
            },
            Scrolling::Top { offset } => offset,
        }
    }

    pub fn set_offset(&mut self, new_offset: Pixels) {
        let content_height = self.content_height_for_scrollbar();
        let new_offset = new_offset.min(Pixels::ZERO);
        let total_offset = -(content_height - self.bounds_y);

        if new_offset < total_offset + self.line_height / 4.0 {
            self.scrolling = Scrolling::Bottom;
        } else {
            self.scrolling = Scrolling::Top { offset: new_offset };
        }
    }
}

#[derive(Clone)]
pub struct ScrollHandler {
    pub state: Rc<RefCell<ScrollState>>,
}

impl ScrollbarHandle for ScrollHandler {
    fn offset(&self) -> Point<Pixels> {
        let state = self.state.borrow();
        Point::new(Pixels::ZERO, state.offset())
    }

    fn set_offset(&self, new_offset: Point<Pixels>) {
        let mut state = self.state.borrow_mut();
        state.set_offset(new_offset.y);
    }

    fn content_size(&self) -> Size<Pixels> {
        let state = self.state.borrow();
        let content_height = state.content_height_for_scrollbar();
        Size::new(Pixels::ZERO, content_height)
    }

    fn start_drag(&self) {
        let mut state = self.state.borrow_mut();
        state.active_drag = Some(ActiveDrag {
            start_content_height: state.lines * state.line_height,
            drag_pivot: Pixels::ZERO,
            real_pivot: Pixels::ZERO,
            actual_offset: state.offset(),
        });
    }

    fn end_drag(&self) {
        let mut state = self.state.borrow_mut();
        if let Some(drag) = state.active_drag.take() {
            state.set_offset(drag.actual_offset);
        }
    }
}

#[derive(Debug)]
pub struct ScrollRenderInfo {
    pub item: usize,
    pub reverse: bool,
    pub offset: Pixels,
}

pub fn update_scrolling<T: ScrollableLine>(
    scroll_state: &mut ScrollState,
    font: &Font,
    items: &mut Vec<T>,
    last_scrolled_item: &mut usize,
    item_sizes: &mut FenwickTree,
    total_line_count: &mut usize,
    cache: &mut dyn WrappedLineCache,
    line_height: Pixels,
    wrap_width: Pixels,
    font_size: Pixels,
    text_style: &TextStyle,
    line_wrapper: &mut LineWrapperHandle,
    text_system: &Arc<WindowTextSystem>,
) -> ScrollRenderInfo {
    if items.is_empty() {
        scroll_state.scrolling = Scrolling::Bottom;
        *last_scrolled_item = 0;
        return ScrollRenderInfo {
            item: 0,
            reverse: false,
            offset: Pixels::ZERO,
        };
    }

    let max_offset = (*total_line_count * line_height - scroll_state.bounds_y).max(px(1.0));

    match &mut scroll_state.scrolling {
        Scrolling::Bottom => {
            if let Some(active_drag) = &mut scroll_state.active_drag {
                active_drag.actual_offset = -max_offset;
            }
            *last_scrolled_item = items.len().saturating_sub(1);
            ScrollRenderInfo {
                item: items.len().saturating_sub(1),
                reverse: true,
                offset: Pixels::ZERO,
            }
        },
        Scrolling::Top { offset } => {
            let mut offset = *offset;

            for check_scrolled_items in [true, false] {
                let mut effective_offset = offset;

                if offset <= -max_offset {
                    scroll_state.scrolling = Scrolling::Bottom;
                    if let Some(active_drag) = &mut scroll_state.active_drag {
                        active_drag.actual_offset = -max_offset;
                    }
                    *last_scrolled_item = items.len().saturating_sub(1);
                    return ScrollRenderInfo {
                        item: items.len().saturating_sub(1),
                        reverse: true,
                        offset: Pixels::ZERO,
                    };
                }

                if offset < px(-1.0)
                    && let Some(active_drag) = &scroll_state.active_drag
                {
                    let drag_pivot = active_drag.drag_pivot.min(Pixels::ZERO);
                    let real_pivot = active_drag.real_pivot.min(Pixels::ZERO);
                    let new_max_offset = (*total_line_count * line_height - scroll_state.bounds_y).max(px(1.0));
                    let old_max_offset = (active_drag.start_content_height - scroll_state.bounds_y).max(px(1.0));

                    if offset < drag_pivot {
                        effective_offset = (offset - drag_pivot) / (-old_max_offset - drag_pivot)
                            * (-new_max_offset - real_pivot)
                            + real_pivot;
                    } else {
                        effective_offset = offset / drag_pivot * real_pivot;
                    }
                }

                if let Some(active_drag) = &mut scroll_state.active_drag {
                    active_drag.actual_offset = effective_offset;
                }

                let top = (-effective_offset).max(Pixels::ZERO);
                let top_offset_for_inset = line_height.min(top);
                let top = top - top_offset_for_inset;

                let top_line = (top / line_height) as usize;
                let line_remainder = top_line * line_height - top;

                let (item_index, remainder_lines) = item_sizes.index_of_with_remainder(top_line + 1);

                if check_scrolled_items && item_index < *last_scrolled_item {
                    let mut resized_above = Pixels::ZERO;
                    let mut changed = false;
                    let from = item_index.max(last_scrolled_item.saturating_sub(32));
                    for item in items[from..*last_scrolled_item].iter_mut() {
                        if item.is_skip() {
                            continue;
                        }
                        let lines = item.compute_wrapped_text(
                            wrap_width,
                            text_system,
                            font,
                            font_size,
                            text_style,
                            line_wrapper,
                            cache,
                        );
                        let line_count = lines.len().max(1);
                        if line_count != item.total_lines() {
                            resized_above += line_count * line_height - item.total_lines() * line_height;
                            if item.total_lines() < line_count {
                                item_sizes.add_at(item.index(), line_count - item.total_lines());
                                *total_line_count += line_count - item.total_lines();
                            } else {
                                item_sizes.sub_at(item.index(), item.total_lines() - line_count);
                                *total_line_count -= item.total_lines() - line_count;
                            }
                            item.set_total_lines(line_count);
                            changed = true;
                        }
                    }
                    if changed {
                        if let Some(active_drag) = &mut scroll_state.active_drag {
                            active_drag.drag_pivot = offset;
                            active_drag.real_pivot = effective_offset - resized_above;
                        } else {
                            offset -= resized_above;
                            if let Scrolling::Top { offset } = &mut scroll_state.scrolling {
                                *offset -= resized_above;
                            }
                        }
                        continue;
                    }
                }

                let render_offset =
                    -(remainder_lines * line_height) + line_remainder + line_height - top_offset_for_inset;

                if scroll_state.active_drag.is_some() {
                    let mut remaining_lines = ((scroll_state.bounds_y - render_offset) / line_height) as usize + 1;
                    let mut changed = false;
                    for item in items[item_index..].iter_mut() {
                        if item.is_skip() {
                            continue;
                        }
                        let lines = item.compute_wrapped_text(
                            wrap_width,
                            text_system,
                            font,
                            font_size,
                            text_style,
                            line_wrapper,
                            cache,
                        );
                        let line_count = lines.len().max(1);
                        if line_count != item.total_lines() {
                            if item.total_lines() < line_count {
                                item_sizes.add_at(item.index(), line_count - item.total_lines());
                                *total_line_count += line_count - item.total_lines();
                            } else {
                                item_sizes.sub_at(item.index(), item.total_lines() - line_count);
                                *total_line_count -= item.total_lines() - line_count;
                            }
                            item.set_total_lines(line_count);
                            changed = true;
                        }
                        remaining_lines = remaining_lines.saturating_sub(line_count);
                        if remaining_lines == 0 {
                            break;
                        }
                    }
                    if changed && let Some(active_drag) = &mut scroll_state.active_drag {
                        active_drag.drag_pivot = offset;
                        active_drag.real_pivot = effective_offset;
                    }
                }

                *last_scrolled_item = item_index;
                return ScrollRenderInfo {
                    item: item_index,
                    reverse: false,
                    offset: render_offset,
                };
            }
            unreachable!();
        },
    }
}
