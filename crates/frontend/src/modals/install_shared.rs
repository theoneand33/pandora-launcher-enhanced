use std::cmp::Ordering;

use enumset::{EnumSet, EnumSetType};
use gpui::{prelude::*, *};
use gpui_component::{
    IndexPath,
    select::{SearchableVec, Select, SelectState},
};
use rustc_hash::FxHashMap;

pub struct VersionMatrixLoaders<L: EnumSetType> {
    pub loaders: EnumSet<L>,
    pub same_loaders_for_all_versions: bool,
}

pub struct LoaderSelection<'a, L: EnumSetType> {
    pub single_loader_set: &'a mut Option<EnumSet<L>>,
    pub force_target_loader: bool,
    pub target_loader_label: Option<SharedString>,
    pub last_selected_loader: &'a Option<SharedString>,
}

fn is_snapshot_version(version: &str) -> bool {
    // Recognize valid snapshot identifiers like "24w03a" (YYwWWx) without matching any
    // string that merely contains 'w'. Preserves existing pre/rc checks separately.
    let bytes = version.as_bytes();
    bytes.len() == 6
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2] == b'w'
        && bytes[3].is_ascii_digit()
        && bytes[4].is_ascii_digit()
        && bytes[5].is_ascii_lowercase()
}

pub fn render_select_minecraft_version<L: EnumSetType, D: 'static>(
    select_state: &mut Option<Entity<SelectState<SearchableVec<SharedString>>>>,
    version_matrix: &FxHashMap<&'static str, VersionMatrixLoaders<L>>,
    fixed_minecraft_version: &Option<&'static str>,
    window: &mut Window,
    cx: &mut Context<D>,
) -> AnyElement {
    let select_state = select_state.get_or_insert_with(|| {
        if let Some(minecraft_version) = *fixed_minecraft_version {
            cx.new(|cx| {
                let mut select_state = SelectState::new(
                    SearchableVec::new(vec![SharedString::new_static(minecraft_version)]),
                    None,
                    window,
                    cx,
                )
                .searchable(true);
                select_state.set_selected_index(Some(IndexPath::default()), window, cx);
                select_state
            })
        } else {
            let mut keys: Vec<SharedString> = version_matrix.keys().cloned().map(SharedString::new_static).collect();
            keys.sort_by(|a, b| {
                let a_is_snapshot = is_snapshot_version(a) || a.contains("pre") || a.contains("rc");
                let b_is_snapshot = is_snapshot_version(b) || b.contains("pre") || b.contains("rc");
                if a_is_snapshot != b_is_snapshot {
                    if a_is_snapshot {
                        Ordering::Greater
                    } else {
                        Ordering::Less
                    }
                } else {
                    lexical_sort::natural_lexical_cmp(a, b).reverse()
                }
            });
            cx.new(|cx| {
                let mut select_state = SelectState::new(SearchableVec::new(keys), None, window, cx).searchable(true);
                select_state.set_selected_index(Some(IndexPath::default()), window, cx);
                select_state
            })
        }
    });

    Select::new(select_state)
        .disabled(fixed_minecraft_version.is_some())
        .title_prefix(format!("{}: ", t::instance::game_version()))
        .search_placeholder(t::common::search())
        .into_any_element()
}

pub fn render_select_loader<L: EnumSetType, D: 'static>(
    select_state: &mut Option<Entity<SelectState<Vec<SharedString>>>>,
    version_matrix: &FxHashMap<&'static str, VersionMatrixLoaders<L>>,
    constraints: LoaderSelection<'_, L>,
    selected_minecraft_version: &SharedString,
    pretty_name: impl Fn(L) -> &'static str,
    window: &mut Window,
    cx: &mut Context<D>,
) -> AnyElement {
    let LoaderSelection {
        single_loader_set,
        force_target_loader,
        target_loader_label,
        last_selected_loader,
    } = constraints;
    let loader_select_state = select_state.get_or_insert_with(|| {
        *single_loader_set = None;

        if let Some(loader) = target_loader_label
            && force_target_loader
        {
            cx.new(|cx| {
                let mut select_state = SelectState::new(vec![loader], None, window, cx);
                select_state.set_selected_index(Some(IndexPath::default()), window, cx);
                select_state
            })
        } else if let Some(loaders) = version_matrix.get(selected_minecraft_version.as_str()) {
            if loaders.same_loaders_for_all_versions {
                let single_loader = if loaders.loaders.len() == 1 {
                    SharedString::new_static(pretty_name(loaders.loaders.iter().next().unwrap()))
                } else {
                    let mut string = String::new();
                    let mut first = true;
                    for loader in loaders.loaders.iter() {
                        if first {
                            first = false;
                        } else {
                            string.push_str(" / ");
                        }
                        string.push_str(pretty_name(loader));
                    }
                    SharedString::new(string)
                };

                *single_loader_set = Some(loaders.loaders);

                cx.new(|cx| {
                    let mut select_state = SelectState::new(vec![single_loader], None, window, cx);
                    select_state.set_selected_index(Some(IndexPath::default()), window, cx);
                    select_state
                })
            } else {
                let keys: Vec<SharedString> =
                    loaders.loaders.iter().map(pretty_name).map(SharedString::new_static).collect();

                cx.new(|cx| {
                    let mut select_state = SelectState::new(keys, None, window, cx);
                    if let Some(previous) = last_selected_loader {
                        select_state.set_selected_value(previous, window, cx);
                    }
                    if select_state.selected_index(cx).is_none() {
                        select_state.set_selected_index(Some(IndexPath::default()), window, cx);
                    }
                    select_state
                })
            }
        } else {
            cx.new(|cx| {
                let mut select_state = SelectState::new(Vec::new(), None, window, cx);
                select_state.set_selected_index(Some(IndexPath::default()), window, cx);
                select_state
            })
        }
    });

    Select::new(loader_select_state)
        .disabled(force_target_loader || single_loader_set.is_some())
        .title_prefix(format!("{}: ", t::instance::loader()))
        .into_any_element()
}
