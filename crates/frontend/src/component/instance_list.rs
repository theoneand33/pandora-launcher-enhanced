use bridge::{handle::BackendHandle, instance::InstanceStatus, message::MessageToBackend};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    table::{Column, ColumnSort, TableDelegate, TableState},
    v_flex,
};

use crate::{
    entity::{
        DataEntities,
        instance::{InstanceAddedEvent, InstanceEntry, InstanceModifiedEvent, InstanceRemovedEvent},
    },
    icon::PandoraIcon,
    interface_config::InterfaceConfig,
    modals, png_render_cache, root, ui,
};

pub struct InstanceList {
    columns: Vec<Column>,
    items: Vec<InstanceEntry>,
    filter: SharedString,
    group_filter: Option<SharedString>,
    visible_cache: Vec<usize>,
    backend_handle: BackendHandle,
    _instance_added_subscription: Subscription,
    _instance_removed_subscription: Subscription,
    _instance_modified_subscription: Subscription,
}

impl InstanceList {
    pub fn create_table(data: &DataEntities, window: &mut Window, cx: &mut App) -> Entity<TableState<Self>> {
        let instances = data.instances.clone();
        let items = instances.read(cx).entries.values().map(|i| i.read(cx).clone()).collect();
        cx.new(|cx| {
            let _instance_added_subscription = cx.subscribe::<_, InstanceAddedEvent>(
                &instances,
                |table: &mut TableState<InstanceList>, _, event, cx| {
                    let d = table.delegate_mut();
                    d.items.insert(0, event.instance.clone());
                    d.rebuild_visible_cache();
                    cx.notify();
                },
            );
            let _instance_removed_subscription =
                cx.subscribe::<_, InstanceRemovedEvent>(&instances, |table, _, event, cx| {
                    let d = table.delegate_mut();
                    d.items.retain(|instance| instance.id != event.id);
                    d.rebuild_visible_cache();
                    cx.notify();
                });
            let _instance_modified_subscription =
                cx.subscribe::<_, InstanceModifiedEvent>(&instances, |table, _, event, cx| {
                    let d = table.delegate_mut();
                    if let Some(entry) = d.items.iter_mut().find(|entry| entry.id == event.instance.id) {
                        *entry = event.instance.clone();
                        d.rebuild_visible_cache();
                        cx.notify();
                    }
                });
            let mut instance_list = Self {
                columns: vec![
                    Column::new("pin", "").width(36.).fixed_left().movable(false).resizable(false),
                    Column::new("controls", "").width(150.).fixed_left().movable(false).resizable(false),
                    Column::new("name", t::instance::name())
                        .width(150.)
                        .fixed_left()
                        .sortable()
                        .resizable(true),
                    Column::new("version", t::instance::version())
                        .width(150.)
                        .fixed_left()
                        .sortable()
                        .resizable(true),
                    Column::new("loader", t::instance::modloader()).width(150.).fixed_left().resizable(true),
                    Column::new("remove", "").width(44.).fixed_left().movable(false).resizable(false),
                ],
                items,
                filter: SharedString::default(),
                group_filter: None,
                visible_cache: Vec::new(),
                backend_handle: data.backend_handle.clone(),
                _instance_added_subscription,
                _instance_removed_subscription,
                _instance_modified_subscription,
            };
            instance_list.rebuild_visible_cache();
            TableState::new(instance_list, window, cx)
        })
    }

    pub fn set_filter(&mut self, query: SharedString) {
        self.filter = query;
        self.rebuild_visible_cache();
    }

    pub fn set_group_filter(&mut self, group: Option<SharedString>) {
        self.group_filter = group;
        self.rebuild_visible_cache();
    }

    pub fn all_groups(&self) -> Vec<SharedString> {
        let mut groups: Vec<SharedString> = self
            .items
            .iter()
            .filter_map(|e| e.configuration.group.map(|g| SharedString::from(g.as_str())))
            .collect();
        groups.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        groups.dedup();
        groups
    }

    fn matches_filter(entry: &InstanceEntry, lower: &str) -> bool {
        if lower.is_empty() {
            return true;
        }
        let name = entry.name.to_lowercase();
        if name.contains(lower) {
            return true;
        }
        let ver = entry.configuration.minecraft_version.as_str().to_lowercase();
        if ver.contains(lower) {
            return true;
        }
        let loader = entry.configuration.loader.pretty_name().to_lowercase();
        if loader.contains(lower) {
            return true;
        }
        if let Some(group) = entry.configuration.group.map(|g| g.as_str().to_lowercase())
            && group.contains(lower)
        {
            return true;
        }
        false
    }

    fn matches_group(entry: &InstanceEntry, group_filter: Option<&str>) -> bool {
        match group_filter {
            None => true,
            Some("__ungrouped__") => entry.configuration.group.is_none(),
            Some(g) => entry.configuration.group.is_some_and(|v| v.as_str() == g),
        }
    }

    fn compute_visible(items: &[InstanceEntry], filter: &str, group_filter: Option<&str>) -> Vec<usize> {
        let lower = filter.to_lowercase();
        let mut out: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, e)| Self::matches_filter(e, &lower) && Self::matches_group(e, group_filter))
            .map(|(i, _)| i)
            .collect();
        // pinned first, stable to preserve order from perform_sort
        out.sort_by(|&a, &b| items[b].configuration.pinned.cmp(&items[a].configuration.pinned));
        out
    }

    fn rebuild_visible_cache(&mut self) {
        let group = self.group_filter.clone();
        self.visible_cache = Self::compute_visible(&self.items, &self.filter, group.as_deref());
    }

    fn visible_indices(&self) -> &[usize] {
        &self.visible_cache
    }

    fn visible_entry(&self, visible_ix: usize) -> Option<&InstanceEntry> {
        let idx = self.visible_indices().get(visible_ix).copied()?;
        self.items.get(idx)
    }

    pub fn total_count(&self) -> usize {
        self.items.len()
    }

    pub fn render_card(&self, index: usize, cx: &mut App) -> Div {
        let Some(item) = self.visible_entry(index) else {
            return v_flex().into();
        };
        let loader_and_version = format!(
            "{} {}",
            item.configuration.loader.pretty_name(),
            item.configuration.minecraft_version.as_str(),
        );

        let icon_element = if let Some(icon) = item.icon.clone() {
            let transform = png_render_cache::ImageTransformation::Resize { width: 64, height: 64 };
            png_render_cache::render_with_transform(icon, transform, cx)
                .rounded(cx.theme().radius)
                .size_16()
                .min_w_16()
                .min_h_16()
                .into_any_element()
        } else {
            let icon_path = item.configuration.instance_fallback_icon.map(|s| s.as_str()).unwrap_or("icons/box.svg");
            Icon::default().path(icon_path).size_16().min_w_16().min_h_16().into_any_element()
        };

        let play_button = render_play_button(item, index, self.backend_handle.clone());

        let theme = cx.theme();
        let id = item.id;
        let name = item.name.clone();
        let backend_handle = self.backend_handle.clone();
        let backend_handle_for_icon = self.backend_handle.clone();
        let backend_handle_for_rename = self.backend_handle.clone();
        let name_for_rename = item.name.clone();
        let trash_icon = PandoraIcon::Trash2;
        let edit_icon = Icon::new(PandoraIcon::Brush).text_color(white());
        let icon_hover_group = format!("instance-icon-edit-{index}");
        let icon_overlay_hover_group = icon_hover_group.clone();
        let icon = div()
            .id(("icon", index))
            .group(icon_hover_group)
            .cursor_pointer()
            .size_16()
            .min_w_16()
            .min_h_16()
            .relative()
            .on_click(move |_, window, cx| {
                let backend_handle = backend_handle_for_icon.clone();
                crate::modals::select_icon::open_select_icon(
                    Box::new(move |icon, _| {
                        backend_handle.send(MessageToBackend::SetInstanceIcon { id, icon: Some(icon) });
                    }),
                    window,
                    cx,
                );
            })
            .child(icon_element)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(black().opacity(0.5))
                    .opacity(0.0)
                    .group_hover(icon_overlay_hover_group, |this| this.opacity(1.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(edit_icon.clone().size_8()),
            );

        v_flex()
            .flex_1()
            .p_2()
            .gap_2()
            .w_full()
            .min_w_64()
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .relative()
            .child({
                let name_hover_group = format!("instance-name-edit-{index}");
                let name_overlay_hover_group = name_hover_group.clone();
                h_flex().w_full().gap_2().child(icon).child(
                    v_flex()
                        .truncate()
                        .w_full()
                        .pr_16()
                        .relative()
                        .child(
                            div()
                                .id(("rename", index))
                                .group(name_hover_group)
                                .cursor_pointer()
                                .w_48()
                                .max_w_full()
                                .pl_5()
                                .on_click(move |_, window, cx| {
                                    modals::rename_instance::open_rename_instance(
                                        id,
                                        name_for_rename.clone(),
                                        backend_handle_for_rename.clone(),
                                        window,
                                        cx,
                                    );
                                })
                                .child(item.name.clone())
                                .child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .bottom_0()
                                        .left_0()
                                        .opacity(0.0)
                                        .group_hover(name_overlay_hover_group, |this| this.opacity(1.0))
                                        .flex()
                                        .items_center()
                                        .justify_start()
                                        .child(edit_icon.clone().size_4()),
                                ),
                        )
                        .child(loader_and_version)
                        .when_some(item.configuration.group, |this, group| {
                            this.child(
                                div()
                                    .text_xs()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(cx.theme().accent.opacity(0.15))
                                    .text_color(cx.theme().accent)
                                    .child(SharedString::from(group.as_str())),
                            )
                        }),
                )
            })
            .child(h_flex().gap_2().child(play_button.flex_1().small()).child(
                Button::new(("view", index)).flex_1().small().info().label(t::instance::view()).on_click({
                    let name = item.name.clone();
                    move |_, window, cx| {
                        root::switch_page(
                            ui::PageType::InstancePage { name: name.clone() },
                            &[ui::PageType::Instances],
                            window,
                            cx,
                        );
                    }
                }),
            ))
            .child(
                h_flex()
                    .absolute()
                    .top_1()
                    .right_1()
                    .gap_1()
                    .child({
                        let pinned = item.configuration.pinned;
                        let bh = self.backend_handle.clone();
                        Button::new(("pin", index))
                            .small()
                            .compact()
                            .icon(if pinned {
                                PandoraIcon::Star
                            } else {
                                PandoraIcon::StarOff
                            })
                            .tooltip(if pinned {
                                t::instance::unpin()
                            } else {
                                t::instance::pin()
                            })
                            .on_click(move |_, _, _| {
                                bh.send(MessageToBackend::SetInstancePinned { id, pinned: !pinned });
                            })
                    })
                    .child(
                        Button::new(("remove", index))
                            .danger()
                            .small()
                            .compact()
                            .icon(trash_icon)
                            .tooltip(t::instance::delete())
                            .on_click(move |click: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                window.prevent_default();
                                if InterfaceConfig::get(cx).quick_delete_instance && click.modifiers().shift {
                                    backend_handle.send(MessageToBackend::DeleteInstance { id });
                                } else {
                                    modals::delete_instance::open_delete_instance(
                                        id,
                                        name.clone(),
                                        backend_handle.clone(),
                                        window,
                                        cx,
                                    );
                                }
                            }),
                    ),
            )
    }
}

impl TableDelegate for InstanceList {
    fn columns_count(&self, _cx: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.visible_indices().len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> gpui_component::table::Column {
        self.columns[col_ix].clone()
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: gpui_component::table::ColumnSort,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) {
        // ponytail: pinned stays on top regardless of sort; sort only within pinned groups
        if let Some(col) = self.columns.get_mut(col_ix) {
            match col.key.as_ref() {
                "name" => self.items.sort_by(|a, b| {
                    if a.configuration.pinned != b.configuration.pinned {
                        return b.configuration.pinned.cmp(&a.configuration.pinned);
                    }
                    match sort {
                        ColumnSort::Descending => lexical_sort::natural_lexical_cmp(&a.name, &b.name).reverse(),
                        _ => lexical_sort::natural_lexical_cmp(&a.name, &b.name),
                    }
                }),
                "version" => self.items.sort_by(|a, b| {
                    if a.configuration.pinned != b.configuration.pinned {
                        return b.configuration.pinned.cmp(&a.configuration.pinned);
                    }
                    match sort {
                        ColumnSort::Descending => lexical_sort::natural_lexical_cmp(
                            &a.configuration.minecraft_version,
                            &b.configuration.minecraft_version,
                        )
                        .reverse(),
                        _ => lexical_sort::natural_lexical_cmp(
                            &a.configuration.minecraft_version,
                            &b.configuration.minecraft_version,
                        ),
                    }
                }),
                _ => {},
            }
            self.rebuild_visible_cache();
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(item) = self.visible_entry(row_ix).cloned() else {
            return div().into_any_element();
        };
        if let Some(col) = self.columns.get(col_ix) {
            match col.key.as_ref() {
                "pin" => {
                    let id = item.id;
                    let pinned = item.configuration.pinned;
                    let bh = self.backend_handle.clone();
                    h_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .child(
                            Button::new(("pin", row_ix))
                                .small()
                                .compact()
                                .icon(if pinned {
                                    PandoraIcon::Star
                                } else {
                                    PandoraIcon::StarOff
                                })
                                .tooltip(if pinned {
                                    t::instance::unpin()
                                } else {
                                    t::instance::pin()
                                })
                                .on_click(move |_, _, _| {
                                    bh.send(MessageToBackend::SetInstancePinned { id, pinned: !pinned });
                                }),
                        )
                        .into_any_element()
                },
                "name" => {
                    let id = item.id;
                    let name = item.name.clone();
                    let backend_handle = self.backend_handle.clone();
                    let edit_icon = Icon::new(PandoraIcon::Brush).text_color(white());
                    let hover_group = format!("instance-list-name-edit-{row_ix}");
                    let overlay_hover_group = hover_group.clone();
                    div()
                        .id(("rename-list", row_ix))
                        .group(hover_group)
                        .relative()
                        .cursor_pointer()
                        .w_full()
                        .pl_5()
                        .on_click(move |_, window, cx| {
                            modals::rename_instance::open_rename_instance(
                                id,
                                name.clone(),
                                backend_handle.clone(),
                                window,
                                cx,
                            );
                        })
                        .child(item.name.clone())
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left_0()
                                .opacity(0.0)
                                .group_hover(overlay_hover_group, |this| this.opacity(1.0))
                                .flex()
                                .items_center()
                                .justify_start()
                                .child(edit_icon.size_4()),
                        )
                        .into_any_element()
                },
                "version" => item.configuration.minecraft_version.as_str().into_any_element(),
                "controls" => {
                    let play_button = render_play_button(&item, row_ix, self.backend_handle.clone());

                    h_flex()
                        .size_full()
                        .gap_2()
                        .border_r_4()
                        .child(play_button.w_1_2().small())
                        .child(Button::new("view").w_1_2().small().info().label(t::instance::view()).on_click({
                            let name = item.name.clone();
                            move |_, window, cx| {
                                root::switch_page(
                                    ui::PageType::InstancePage { name: name.clone() },
                                    &[ui::PageType::Instances],
                                    window,
                                    cx,
                                );
                            }
                        }))
                        .into_any_element()
                },
                "loader" => item.configuration.loader.pretty_name().into_any_element(),
                "remove" => {
                    let backend_handle = self.backend_handle.clone();
                    let id = item.id;
                    let name = item.name.clone();
                    let trash_icon = PandoraIcon::Trash2;
                    h_flex()
                        .size_full()
                        .items_center()
                        .child(
                            Button::new(("remove", row_ix))
                                .danger()
                                .small()
                                .compact()
                                .icon(trash_icon)
                                .tooltip(t::instance::delete())
                                .on_click(move |click: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    window.prevent_default();
                                    if InterfaceConfig::get(cx).quick_delete_instance && click.modifiers().shift {
                                        backend_handle.send(MessageToBackend::DeleteInstance { id });
                                    } else {
                                        modals::delete_instance::open_delete_instance(
                                            id,
                                            name.clone(),
                                            backend_handle.clone(),
                                            window,
                                            cx,
                                        );
                                    }
                                }),
                        )
                        .into_any_element()
                },
                _ => t::common::unknown().into_any_element(),
            }
        } else {
            t::common::unknown().into_any_element()
        }
    }
}

fn render_play_button(item: &InstanceEntry, index: usize, backend_handle: BackendHandle) -> Button {
    let name = item.name.clone();
    let id = item.id;
    match item.status {
        InstanceStatus::NotRunning => Button::new(("start_instance", index))
            .success()
            .label(t::instance::start::label())
            .on_click(move |_, window, cx| {
                root::start_instance(id, name.clone(), None, &backend_handle, window, cx);
            }),
        InstanceStatus::Launching => Button::new(("launching", index)).warning().label(t::instance::start::starting()),
        InstanceStatus::Stopping => Button::new(("stopping", index)).danger().label(t::instance::start::stopping()),
        InstanceStatus::Running => {
            Button::new(("kill_instance", index)).danger().label(t::instance::kill()).on_click({
                let backend_handle = backend_handle.clone();
                move |_, _, _| {
                    backend_handle.send(MessageToBackend::KillInstance { id });
                }
            })
        },
    }
}
