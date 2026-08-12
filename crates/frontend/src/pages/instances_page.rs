use bridge::handle::BackendHandle;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Icon, IndexPath, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectDelegate, SelectEvent, SelectItem, SelectState},
    table::{DataTable, TableDelegate, TableState},
    v_flex,
};
use strum::IntoEnumIterator;

use crate::{
    component::{
        instance_list::{GroupFilter, InstanceList},
        named_dropdown::{NamedDropdown, NamedDropdownItem},
        responsive_grid::ResponsiveGrid,
    },
    entity::{DataEntities, instance::InstanceEntries, metadata::FrontendMetadata},
    icon::PandoraIcon,
    interface_config::{InstancesViewMode, InterfaceConfig},
    pages::page::Page,
};

pub struct InstancesPage {
    instance_table: Entity<TableState<InstanceList>>,
    view_dropdown: Entity<SelectState<NamedDropdown<InstancesViewMode>>>,
    group_dropdown: Entity<SelectState<NamedDropdown<GroupFilter>>>,
    search_state: Entity<InputState>,

    metadata: Entity<FrontendMetadata>,
    instances: Entity<InstanceEntries>,

    backend_handle: BackendHandle,
}

impl InstancesPage {
    pub fn new(data: &DataEntities, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let instance_table = InstanceList::create_table(data, window, cx);
        let view_dropdown = cx.new(|cx| {
            let items = InstancesViewMode::iter()
                .map(|view| NamedDropdownItem {
                    name: view.name(),
                    item: view,
                })
                .collect::<Vec<_>>();
            let current_view = InterfaceConfig::get(cx).instances_view_mode;
            let row = items.iter().position(|v| v.item == current_view).unwrap_or(0);
            let delegate = NamedDropdown::new(items);
            SelectState::new(delegate, Some(IndexPath::new(row)), window, cx)
        });
        cx.subscribe(&view_dropdown, |_, _, event: &SelectEvent<NamedDropdown<InstancesViewMode>>, cx| {
            let SelectEvent::Confirm(Some(value)) = event else {
                return;
            };
            let view = value.item;

            InterfaceConfig::get_mut(cx).instances_view_mode = view;
        })
        .detach();

        let search_state = cx.new(|cx| InputState::new(window, cx).placeholder(t::common::search()));
        let table_for_search = instance_table.clone();
        cx.subscribe(&search_state, move |_, state, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                let q: SharedString = state.read(cx).value().into();
                table_for_search.update(cx, |table, cx| {
                    table.delegate_mut().set_filter(q);
                    cx.notify();
                });
                cx.notify();
            }
        })
        .detach();

        let group_dropdown = cx.new(|cx| {
            let items = Self::build_group_items(data.instances.read(cx), cx);
            SelectState::new(NamedDropdown::new(items), Some(IndexPath::new(0)), window, cx)
        });
        let table_for_group = instance_table.clone();
        cx.subscribe(&group_dropdown, move |_, _, event: &SelectEvent<NamedDropdown<GroupFilter>>, cx| {
            let SelectEvent::Confirm(value) = event;
            let Some(v) = value.as_ref() else {
                return;
            };
            let filter = v.item.clone();
            table_for_group.update(cx, |table, cx| {
                table.delegate_mut().set_group_filter(filter);
                cx.notify();
            });
        })
        .detach();

        Self::wire_group_refresh(&data.instances, group_dropdown.downgrade(), instance_table.downgrade(), window, cx);

        Self {
            instance_table,
            view_dropdown,
            group_dropdown,
            search_state,
            metadata: data.metadata.clone(),
            instances: data.instances.clone(),
            backend_handle: data.backend_handle.clone(),
        }
    }
}

fn create_instance_button(
    id: impl Into<SharedString>,
    metadata: Entity<FrontendMetadata>,
    instances: Entity<InstanceEntries>,
    backend_handle: BackendHandle,
) -> Button {
    Button::new(id.into())
        .success()
        .icon(PandoraIcon::Plus)
        .label(t::instance::create())
        .on_click({
            move |_, window, cx| {
                crate::modals::create_instance::open_create_instance(
                    metadata.clone(),
                    instances.clone(),
                    backend_handle.clone(),
                    window,
                    cx,
                );
            }
        })
}

impl InstancesPage {
    fn build_group_items(entries: &InstanceEntries, cx: &App) -> Vec<NamedDropdownItem<GroupFilter>> {
        let mut items = vec![
            NamedDropdownItem {
                name: t::instance::group::all().into(),
                item: GroupFilter::All,
            },
            NamedDropdownItem {
                name: t::instance::group::ungrouped().into(),
                item: GroupFilter::Ungrouped,
            },
        ];
        let groups = InstanceList::deduped_groups(
            entries
                .entries
                .values()
                .filter_map(|e| e.read(cx).configuration.group.as_ref().map(|g| SharedString::from(g.as_ref()))),
        );
        for g in groups {
            // ponytail: group named "ungrouped" would read identically to filter item; disambiguate
            let name = if g.eq_ignore_ascii_case("ungrouped") {
                SharedString::from(format!("\"{}\"", g))
            } else {
                g.clone()
            };
            items.push(NamedDropdownItem {
                name,
                item: GroupFilter::Named(g),
            });
        }
        items
    }

    fn refresh_group_dropdown(
        weak_group: &WeakEntity<SelectState<NamedDropdown<GroupFilter>>>,
        weak_table: &WeakEntity<TableState<InstanceList>>,
        entries: &Entity<InstanceEntries>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(group_state) = weak_group.upgrade() else {
            return;
        };
        let prev_selected: Option<GroupFilter> = group_state.read(cx).selected_value().map(|v| v.item.clone());
        group_state.update(cx, |state, cx| {
            let items = Self::build_group_items(entries.read(cx), cx);
            let group_eq = |a: &GroupFilter, b: &GroupFilter| match (a, b) {
                (GroupFilter::All, GroupFilter::All) => true,
                (GroupFilter::Ungrouped, GroupFilter::Ungrouped) => true,
                (GroupFilter::Named(x), GroupFilter::Named(y)) => x.eq_ignore_ascii_case(y),
                _ => false,
            };
            let prev_idx = prev_selected
                .as_ref()
                .and_then(|prev| items.iter().position(|it| group_eq(&it.item, prev)));
            state.set_items(NamedDropdown::new(items), window, cx);
            if let Some(idx) = prev_idx {
                state.set_selected_index(Some(IndexPath::new(idx)), window, cx);
            } else {
                state.set_selected_index(Some(IndexPath::new(0)), window, cx);
                if let Some(table) = weak_table.upgrade() {
                    table.update(cx, |t, cx| {
                        t.delegate_mut().set_group_filter(GroupFilter::All);
                        cx.notify();
                    });
                }
            }
        });
    }

    fn wire_group_refresh(
        instances: &Entity<InstanceEntries>,
        weak_group: WeakEntity<SelectState<NamedDropdown<GroupFilter>>>,
        weak_table: WeakEntity<TableState<InstanceList>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.subscribe_in(
            instances,
            window,
            move |_, e, _: &crate::entity::instance::InstanceGroupsChangedEvent, window, cx| {
                Self::refresh_group_dropdown(&weak_group, &weak_table, &e, window, cx)
            },
        )
        .detach();
    }
}

impl Page for InstancesPage {
    fn controls(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let create_instance = create_instance_button(
            "create_instance",
            self.metadata.clone(),
            self.instances.clone(),
            self.backend_handle.clone(),
        );
        // wrapping in div makes it not take up the full space of the titlebar
        let select_view =
            div().child(Select::new(&self.view_dropdown).title_prefix(format!("{}: ", t::instance::view())));
        let select_group =
            div().child(Select::new(&self.group_dropdown).title_prefix(format!("{}: ", t::instance::group::label())));

        h_flex()
            .gap_3()
            .child(create_instance)
            .child(
                div()
                    .w_64()
                    .child(Input::new(&self.search_state).small().prefix(Icon::new(PandoraIcon::Search))),
            )
            .child(select_group)
            .child(select_view)
    }

    fn scrollable(&self, cx: &App) -> bool {
        match InterfaceConfig::get(cx).instances_view_mode {
            InstancesViewMode::Cards => true,
            InstancesViewMode::List => false,
        }
    }
}

impl Render for InstancesPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // ponytail: empty check uses total count; filtered empty shows "no results" below.
        let is_empty = self.instance_table.read(cx).delegate().total_count() == 0;
        let is_filtered_empty = !is_empty && self.instance_table.read(cx).delegate().rows_count(cx) == 0;
        if is_filtered_empty {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p_8()
                .child(
                    v_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            Icon::new(crate::icon::PandoraIcon::Search)
                                .size_12()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(div().text_lg().text_color(cx.theme().muted_foreground).child(t::common::no_matches())),
                )
                .into_any_element();
        }
        if is_empty {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p_8()
                .child(
                    v_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            Icon::new(crate::icon::PandoraIcon::Box).size_12().text_color(cx.theme().muted_foreground),
                        )
                        .child(div().text_lg().text_color(cx.theme().muted_foreground).child(t::instance::empty()))
                        .child(create_instance_button(
                            "create_instance_empty",
                            self.metadata.clone(),
                            self.instances.clone(),
                            self.backend_handle.clone(),
                        )),
                )
                .into_any_element();
        }
        match InterfaceConfig::get(cx).instances_view_mode {
            InstancesViewMode::Cards => {
                let cards = self.instance_table.update(cx, |table, cx| {
                    let rows = table.delegate().rows_count(cx);
                    (0..rows).map(|i| table.delegate().render_card(i, cx)).collect::<Vec<_>>()
                });

                let size = Size::new(gpui::AvailableSpace::MinContent, gpui::AvailableSpace::MinContent);

                div()
                    .p_4()
                    .child(ResponsiveGrid::new(size).size_full().gap_4().children(cards))
                    .into_any_element()
            },
            InstancesViewMode::List => DataTable::new(&self.instance_table).bordered(false).into_any_element(),
        }
    }
}

#[derive(Default)]
pub struct VersionList {
    pub versions: Vec<SharedString>,
    pub matched_versions: Vec<SharedString>,
}

impl SelectDelegate for VersionList {
    type Item = SharedString;

    fn items_count(&self, _section: usize) -> usize {
        self.matched_versions.len()
    }

    fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
        self.matched_versions.get(ix.row)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Self::Item: gpui_component::select::SelectItem<Value = V>,
        V: PartialEq,
    {
        for (ix, item) in self.matched_versions.iter().enumerate() {
            if item.value() == value {
                return Some(IndexPath::default().row(ix));
            }
        }

        None
    }

    fn perform_search(&mut self, query: &str, _window: &mut Window, _: &mut App) -> Task<()> {
        let lower_query = query.to_lowercase();

        self.matched_versions = self
            .versions
            .iter()
            .filter(|item| item.to_lowercase().starts_with(&lower_query))
            .cloned()
            .collect();

        Task::ready(())
    }
}
