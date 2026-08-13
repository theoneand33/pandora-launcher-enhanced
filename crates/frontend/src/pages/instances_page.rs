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
        instance_list::InstanceList,
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

        Self {
            instance_table,
            view_dropdown,
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

        let backend_for_join = self.backend_handle.clone();
        let join_p2p = Button::new("join_p2p")
            .icon(PandoraIcon::Download)
            .label(t::instance::p2p::join_title())
            .on_click(move |_, window, cx| {
                crate::modals::p2p_join::open_p2p_join(backend_for_join.clone(), window, cx);
            });

        h_flex()
            .gap_3()
            .child(create_instance)
            .child(join_p2p)
            .child(
                div()
                    .w_64()
                    .child(Input::new(&self.search_state).small().prefix(Icon::new(PandoraIcon::Search))),
            )
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
