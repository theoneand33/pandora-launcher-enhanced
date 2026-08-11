use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use bridge::{
    handle::BackendHandle,
    install::{ContentDownload, ContentInstall, ContentInstallFile, InstallTarget},
    instance::{ContentFolder, InstanceContentSummary, InstanceID},
    modal_action::ModalAction,
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme as _, Disableable, IndexPath, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::SelectAll,
    list::ListState,
    notification::{Notification, NotificationType},
    select::{Select, SelectEvent, SelectState},
    switch::Switch,
    v_flex,
};
use schema::{
    content::{ContentInstallReason, ContentSource},
    curseforge::CurseforgeClassId,
    loader::Loader,
    modrinth::ModrinthProjectType,
};
use ustr::Ustr;

use crate::{
    component::{
        content_list::ContentListDelegate,
        named_dropdown::{NamedDropdown, NamedDropdownItem},
    },
    entity::instance::{ContentStates, InstanceEntry},
    interface_config::{InstanceContentSortKey, InterfaceConfig},
    root,
    ui::PageType,
};

pub struct InstanceContentSubpage {
    content_type: ContentType,
    instance: InstanceID,
    instance_loader: Loader,
    instance_version: Ustr,
    instance_name: SharedString,
    backend_handle: BackendHandle,
    content_states: ContentStates,
    content_list: Entity<ListState<ContentListDelegate>>,
    content: Entity<Arc<[InstanceContentSummary]>>,
    // For non-Mods tabs: the mods folder, so bundled modpack resource-pack/shader children can be
    // surfaced under their own tab. `None` for the Mods tab.
    mods_content: Option<Entity<Arc<[InstanceContentSummary]>>>,
    sort_dropdown: Entity<SelectState<NamedDropdown<InstanceContentSortKey>>>,
    update_check_action: Option<ModalAction>,
    update_all_action: Option<ModalAction>,
    show_outdated_only: bool,
    _add_from_file_task: Option<Task<()>>,
}

/// Combines a tab's own folder content with the modpack bundles from the mods folder (when
/// provided), so a modpack's resource-pack/shader children can appear under the matching tab. The
/// delegate then filters each modpack's children down to the ones destined for its folder.
fn combined_content_of(
    folder: &Entity<Arc<[InstanceContentSummary]>>,
    mods: Option<&Entity<Arc<[InstanceContentSummary]>>>,
    cx: &App,
) -> Vec<InstanceContentSummary> {
    let mut combined: Vec<InstanceContentSummary> = folder.read(cx).to_vec();
    if let Some(mods) = mods {
        for item in mods.read(cx).iter() {
            if item.content_summary.extra.is_modpack() {
                combined.push(item.clone());
            }
        }
    }
    combined
}

fn is_updatable(s: &InstanceContentSummary, loader: Loader, version: &'static str) -> bool {
    s.update.can_update(loader, version)
}

#[derive(Clone, Copy)]
pub enum ContentType {
    Mods,
    ResourcePacks,
    Shaders,
}

impl ContentType {
    fn content_folder(self) -> ContentFolder {
        match self {
            ContentType::Mods => ContentFolder::Mods,
            ContentType::ResourcePacks => ContentFolder::ResourcePacks,
            ContentType::Shaders => ContentFolder::Shaders,
        }
    }

    fn title(self) -> &'static str {
        match self {
            ContentType::Mods => t::instance::content::mods(),
            ContentType::ResourcePacks => t::instance::content::resourcepacks(),
            ContentType::Shaders => t::instance::content::shaders(),
        }
    }

    fn install_select(self) -> &'static str {
        match self {
            ContentType::Mods => t::instance::content::install::select_mods(),
            ContentType::ResourcePacks => t::instance::content::install::select_resourcepacks(),
            ContentType::Shaders => t::instance::content::install::select_shaders(),
        }
    }

    fn modrinth_project_type(self) -> ModrinthProjectType {
        match self {
            ContentType::Mods => ModrinthProjectType::Mod,
            ContentType::ResourcePacks => ModrinthProjectType::Resourcepack,
            ContentType::Shaders => ModrinthProjectType::Shader,
        }
    }

    fn curseforge_class_id(self) -> CurseforgeClassId {
        match self {
            ContentType::Mods => CurseforgeClassId::Mod,
            ContentType::ResourcePacks => CurseforgeClassId::Resourcepack,
            ContentType::Shaders => CurseforgeClassId::Shader,
        }
    }

    fn valid_sort_modes(self) -> &'static [InstanceContentSortKey] {
        match self {
            ContentType::Mods => &[
                InstanceContentSortKey::Name,
                InstanceContentSortKey::ModId,
                InstanceContentSortKey::Filename,
                InstanceContentSortKey::ModifiedTime,
                InstanceContentSortKey::FileSize,
            ],
            ContentType::ResourcePacks | ContentType::Shaders => &[
                InstanceContentSortKey::Filename,
                InstanceContentSortKey::ModifiedTime,
                InstanceContentSortKey::FileSize,
            ],
        }
    }

    fn sort_key(self, config: &InterfaceConfig) -> InstanceContentSortKey {
        match self {
            ContentType::Mods => config.instance_mods_sort_key,
            ContentType::ResourcePacks => config.instance_resourcepacks_sort_key,
            ContentType::Shaders => config.instance_shaders_sort_key,
        }
    }

    fn sort_enabled_first(self, config: &InterfaceConfig) -> bool {
        match self {
            ContentType::Mods => config.instance_mods_sort_enabled_first,
            ContentType::ResourcePacks => config.instance_resourcepacks_sort_enabled_first,
            ContentType::Shaders => config.instance_shaders_sort_enabled_first,
        }
    }

    fn set_sort_key(self, config: &mut InterfaceConfig, value: InstanceContentSortKey) {
        match self {
            ContentType::Mods => config.instance_mods_sort_key = value,
            ContentType::ResourcePacks => config.instance_resourcepacks_sort_key = value,
            ContentType::Shaders => config.instance_shaders_sort_key = value,
        }
    }

    fn set_sort_enabled_first(self, config: &mut InterfaceConfig, value: bool) {
        match self {
            ContentType::Mods => config.instance_mods_sort_enabled_first = value,
            ContentType::ResourcePacks => config.instance_resourcepacks_sort_enabled_first = value,
            ContentType::Shaders => config.instance_shaders_sort_enabled_first = value,
        }
    }
}

impl InstanceContentSubpage {
    pub fn new(
        instance: &Entity<InstanceEntry>,
        content_type: ContentType,
        backend_handle: BackendHandle,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let instance = instance.read(cx);
        let instance_loader = instance.configuration.loader;
        let instance_version = instance.configuration.minecraft_version;
        let instance_id = instance.id;
        let instance_name = instance.name.clone();
        let content_states = instance.content_states.clone();

        let content_folder = content_type.content_folder();
        let content = instance.content[content_folder].clone();
        // Non-Mods tabs also read the mods folder so they can surface modpack-bundled content.
        let mods_content =
            (content_folder != ContentFolder::Mods).then(|| instance.content[ContentFolder::Mods].clone());

        let config = InterfaceConfig::get(cx);
        let mut sort_key = content_type.sort_key(config);
        let enabled_first = content_type.sort_enabled_first(config);

        let valid_sort_modes = content_type.valid_sort_modes();
        if !valid_sort_modes.contains(&sort_key) {
            sort_key = valid_sort_modes[0];
        }

        let mut content_list_delegate = ContentListDelegate::new(
            instance_id,
            content_folder,
            backend_handle.clone(),
            instance_loader,
            instance_version,
            sort_key,
            enabled_first,
        );
        content_list_delegate.set_content(&combined_content_of(&content, mods_content.as_ref(), cx));

        let sort_dropdown = cx.new(|cx| {
            let items = valid_sort_modes
                .iter()
                .map(|key| NamedDropdownItem {
                    name: key.name(),
                    item: *key,
                })
                .collect::<Vec<_>>();

            let row = items.iter().position(|v| v.item == sort_key).unwrap_or(0);
            SelectState::new(NamedDropdown::new(items), Some(IndexPath::new(row)), window, cx)
        });

        let content_for_observe = content.clone();
        let mods_for_observe = mods_content.clone();
        let content_list = cx.new(move |cx| {
            {
                let content_e = content_for_observe.clone();
                let mods_e = mods_for_observe.clone();
                cx.observe(&content_for_observe, move |list: &mut ListState<ContentListDelegate>, _, cx| {
                    list.delegate_mut().set_content(&combined_content_of(&content_e, mods_e.as_ref(), cx));
                    cx.notify();
                })
                .detach();
            }

            if let Some(mods_e) = mods_for_observe.clone() {
                let content_e = content_for_observe.clone();
                let mods_for_closure = mods_e.clone();
                cx.observe(&mods_e, move |list: &mut ListState<ContentListDelegate>, _, cx| {
                    list.delegate_mut()
                        .set_content(&combined_content_of(&content_e, Some(&mods_for_closure), cx));
                    cx.notify();
                })
                .detach();
            }

            ListState::new(content_list_delegate, window, cx).selectable(false).searchable(true)
        });

        cx.subscribe(
            &sort_dropdown,
            |this, _, event: &SelectEvent<NamedDropdown<InstanceContentSortKey>>, cx| {
                let SelectEvent::Confirm(Some(value)) = event else {
                    return;
                };

                let sort_key = value.item;
                let config = InterfaceConfig::get_mut(cx);

                if this.content_type.sort_key(config) == sort_key {
                    return;
                }

                let enabled_first = this.content_type.sort_enabled_first(config);
                this.content_type.set_sort_key(config, sort_key);

                let content = combined_content_of(&this.content, this.mods_content.as_ref(), cx);
                let content_list = this.content_list.clone();
                cx.update_entity(&content_list, |list, cx| {
                    list.delegate_mut().set_sort_options(sort_key, enabled_first);
                    list.delegate_mut().set_content(&content);
                    cx.notify();
                });
                cx.notify();
            },
        )
        .detach();

        Self {
            content_type,
            instance: instance_id,
            instance_loader,
            instance_version,
            instance_name,
            backend_handle,
            content_states,
            content_list,
            content,
            mods_content,
            sort_dropdown,
            update_check_action: None,
            update_all_action: None,
            show_outdated_only: false,
            _add_from_file_task: None,
        }
    }

    fn install_paths(&self, paths: &[PathBuf], window: &mut Window, cx: &mut App) {
        let content_folder = self.content_type.content_folder().folder_name();

        let content_install = ContentInstall {
            target: InstallTarget::Instance(self.instance),
            loader: self.instance_loader,
            minecraft_version: self.instance_version,
            files: paths
                .into_iter()
                .filter_map(|path| {
                    Some(ContentInstallFile {
                        replace_old: None,
                        path: bridge::install::ContentInstallPath::Raw(
                            Path::new(content_folder).join(path.file_name()?).into(),
                        ),
                        download: ContentDownload::File { path: path.clone() },
                        content_source: ContentSource::Manual,
                        reason: ContentInstallReason::Standalone,
                    })
                })
                .collect(),
        };
        crate::root::start_install(content_install, &self.backend_handle, window, cx);
    }
}

impl Render for InstanceContentSubpage {
    fn render(&mut self, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        let theme = cx.theme();

        self.content_states.observe(self.content_type.content_folder());
        // Non-Mods tabs also surface modpack-bundled content, so make sure the mods folder loads.
        if self.mods_content.is_some() {
            self.content_states.observe(ContentFolder::Mods);
        }

        // Poll ModalActions so they reset on completion (both success and error) and
        // drive `request_animation_frame` while active. Tidy up finished actions
        // instead of holding them forever.
        let is_checking = self.update_check_action.as_ref().is_some_and(|a| a.get_finished_at().is_none());
        if is_checking {
            window.request_animation_frame();
        } else if self.update_check_action.as_ref().is_some_and(|a| a.get_finished_at().is_some()) {
            self.update_check_action = None;
        }

        let is_updating_all = self.update_all_action.as_ref().is_some_and(|a| a.get_finished_at().is_none());
        if is_updating_all {
            window.request_animation_frame();
        } else if self.update_all_action.as_ref().is_some_and(|a| a.get_finished_at().is_some()) {
            self.update_all_action = None;
        }

        let combined = combined_content_of(&self.content, self.mods_content.as_ref(), cx);
        // Single source of truth for updatable items — used for both visibility and click handler.
        let updatable: Vec<InstanceContentSummary> = combined
            .iter()
            .filter(|s| is_updatable(s, self.instance_loader, self.instance_version.as_str()))
            .cloned()
            .collect();
        let has_updates = !updatable.is_empty();

        let header = h_flex()
            .gap_3()
            .mb_1()
            .ml_1()
            .child(div().text_lg().child(self.content_type.title()))
            .child(
                Button::new("update")
                    .label(t::instance::content::update::check::label(false))
                    .success()
                    .compact()
                    .small()
                    .loading(is_checking)
                    .when(is_checking, |b| b.disabled(true))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.update_check_action.as_ref().is_some_and(|a| a.get_finished_at().is_none()) {
                            return;
                        }
                        let action = ModalAction::default();
                        this.update_check_action = Some(action.clone());
                        cx.notify();
                        crate::root::start_update_check_with_action(
                            this.instance,
                            &this.backend_handle,
                            window,
                            cx,
                            action,
                        );
                    })),
            )
            .when(has_updates && !is_checking, |this| {
                this.child(
                    Button::new("update-all")
                        .label(t::instance::content::update::all())
                        .success()
                        .compact()
                        .small()
                        .loading(is_updating_all)
                        .when(is_updating_all, |b| b.disabled(true))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.update_all_action.as_ref().is_some_and(|a| a.get_finished_at().is_none()) {
                                return;
                            }
                            let ids = combined_content_of(&this.content, this.mods_content.as_ref(), cx)
                                .into_iter()
                                .filter(|s| is_updatable(s, this.instance_loader, this.instance_version.as_str()))
                                .map(|s| s.id)
                                .collect::<Vec<_>>();
                            if ids.is_empty() {
                                return;
                            }
                            let action =
                                crate::root::update_multiple_mods(this.instance, ids, &this.backend_handle, window, cx);
                            this.update_all_action = Some(action);
                            cx.notify();
                        })),
                )
            })
            .child(
                Button::new("addmr")
                    .label(t::instance::content::install::from_modrinth())
                    .success()
                    .compact()
                    .small()
                    .on_click({
                        let instance_name = self.instance_name.clone();
                        let project_type = self.content_type.modrinth_project_type();
                        move |_, window, cx| {
                            let page = crate::ui::PageType::Modrinth {
                                installing_for: Some(instance_name.clone()),
                            };
                            InterfaceConfig::get_mut(cx).modrinth_page_project_type = project_type;
                            let path = &[
                                PageType::Instances,
                                PageType::InstancePage {
                                    name: instance_name.clone(),
                                },
                            ];
                            root::switch_page(page, path, window, cx);
                        }
                    }),
            )
            .child(
                Button::new("addcf")
                    .label(t::instance::content::install::from_curseforge())
                    .success()
                    .compact()
                    .small()
                    .on_click({
                        let instance_name = self.instance_name.clone();
                        let class_id = self.content_type.curseforge_class_id();
                        move |_, window, cx| {
                            let page = crate::ui::PageType::Curseforge {
                                installing_for: Some(instance_name.clone()),
                            };
                            InterfaceConfig::get_mut(cx).curseforge_page_class_id = class_id;
                            let path = &[
                                PageType::Instances,
                                PageType::InstancePage {
                                    name: instance_name.clone(),
                                },
                            ];
                            root::switch_page(page, path, window, cx);
                        }
                    }),
            )
            .child(
                Button::new("addfile")
                    .label(t::instance::content::install::from_file())
                    .success()
                    .compact()
                    .small()
                    .on_click({
                        cx.listener(move |this, _, window, cx| {
                            let receiver = cx.prompt_for_paths(PathPromptOptions {
                                files: true,
                                directories: false,
                                multiple: true,
                                prompt: Some(this.content_type.install_select().into()),
                            });

                            let entity = cx.entity();
                            let add_from_file_task = window.spawn(cx, async move |cx| {
                                let Ok(result) = receiver.await else {
                                    return;
                                };
                                _ = cx.update_window_entity(&entity, move |this, window, cx| match result {
                                    Ok(Some(paths)) => {
                                        this.install_paths(&paths, window, cx);
                                    },
                                    Ok(None) => {},
                                    Err(error) => {
                                        let error = format!("{}", error);
                                        let notification = Notification::new()
                                            .autohide(false)
                                            .with_type(NotificationType::Error)
                                            .title(error);
                                        window.push_notification(notification, cx);
                                    },
                                });
                            });
                            this._add_from_file_task = Some(add_from_file_task);
                        })
                    }),
            );

        let filter_bar_controls = h_flex()
            .cursor_default()
            .block_mouse_except_scroll()
            .gap_3()
            .items_center()
            .child(
                div().child(
                    Select::new(&self.sort_dropdown)
                        .small()
                        .title_prefix(format!("{}: ", t::instance::content::sort())),
                ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(div().text_sm().child(t::instance::content::enabled_first()))
                    .child(
                        Switch::new("enabled_first")
                            .checked(self.content_type.sort_enabled_first(InterfaceConfig::get(cx)))
                            .on_click(cx.listener(|this, checked, _, cx| {
                                let config = InterfaceConfig::get_mut(cx);
                                let enabled_first = *checked;

                                if this.content_type.sort_enabled_first(config) == enabled_first {
                                    return;
                                }

                                let sort_key = this.content_type.sort_key(config);
                                this.content_type.set_sort_enabled_first(config, enabled_first);

                                let content = combined_content_of(&this.content, this.mods_content.as_ref(), cx);
                                let content_list = this.content_list.clone();
                                cx.update_entity(&content_list, |list, cx| {
                                    list.delegate_mut().set_sort_options(sort_key, enabled_first);
                                    list.delegate_mut().set_content(&content);
                                    cx.notify();
                                });
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(div().text_sm().child(t::instance::content::outdated_only()))
                    .child(Switch::new("outdated_only").checked(self.show_outdated_only).on_click(cx.listener(
                        |this, checked, _, cx| {
                            if this.show_outdated_only == *checked {
                                return;
                            }
                            this.show_outdated_only = *checked;
                            let content = combined_content_of(&this.content, this.mods_content.as_ref(), cx);
                            let cl = this.content_list.clone();
                            cx.update_entity(&cl, |list, cx| {
                                list.delegate_mut().set_outdated_only(*checked);
                                list.delegate_mut().set_content(&content);
                                cx.notify();
                            });
                            cx.notify();
                        },
                    ))),
            )
            .child({
                let has_search = self.content_list.read(cx).delegate().has_search_filter();
                let label = if self.show_outdated_only {
                    t::instance::content::enable_outdated()
                } else if has_search {
                    t::instance::content::enable_filtered()
                } else {
                    t::instance::content::enable_all()
                };
                let tooltip = if has_search {
                    t::instance::content::enable_filtered()
                } else if self.show_outdated_only {
                    t::instance::content::enable_outdated()
                } else {
                    t::instance::content::enable_all()
                };
                Button::new("enable_all")
                    .small()
                    // content_ids() is tab-scoped and respects search/outdated filters:
                    // "Enable all" affects only visible summaries (current tab, filtered
                    // by search query and outdated-only).
                    .tooltip(tooltip)
                    .label(label)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let ids = this.content_list.read(cx).delegate().content_ids();
                        if ids.is_empty() {
                            return;
                        }
                        this.backend_handle.send(bridge::message::MessageToBackend::SetContentEnabled {
                            id: this.instance,
                            content_ids: ids,
                            enabled: true,
                        });
                    }))
            })
            .child({
                let has_search = self.content_list.read(cx).delegate().has_search_filter();
                let label = if self.show_outdated_only {
                    t::instance::content::disable_outdated()
                } else if has_search {
                    t::instance::content::disable_filtered()
                } else {
                    t::instance::content::disable_all()
                };
                let tooltip = if has_search {
                    t::instance::content::disable_filtered()
                } else if self.show_outdated_only {
                    t::instance::content::disable_outdated()
                } else {
                    t::instance::content::disable_all()
                };
                Button::new("disable_all").small().tooltip(tooltip).label(label).on_click(cx.listener(
                    |this, _, _, cx| {
                        let ids = this.content_list.read(cx).delegate().content_ids();
                        if ids.is_empty() {
                            return;
                        }
                        this.backend_handle.send(bridge::message::MessageToBackend::SetContentEnabled {
                            id: this.instance,
                            content_ids: ids,
                            enabled: false,
                        });
                    },
                ))
            })
            .absolute()
            .top(px(4.0))
            .right(px(12.0));

        v_flex().p_4().size_full().child(header).child(
            div()
                .id("content-list-area")
                .relative()
                .drag_over(|style, _: &ExternalPaths, _, cx| style.bg(cx.theme().accent))
                .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                    this.install_paths(paths.paths(), window, cx);
                }))
                .size_full()
                .border_1()
                .rounded(theme.radius)
                .border_color(theme.border)
                .child(self.content_list.clone())
                .child(filter_bar_controls)
                .on_click({
                    let content_list = self.content_list.clone();
                    move |_, _, cx| {
                        cx.update_entity(&content_list, |list, cx| {
                            list.delegate_mut().clear_selection();
                            cx.notify();
                        })
                    }
                })
                .key_context("Input")
                .on_action({
                    let content_list = self.content_list.clone();
                    move |_: &SelectAll, _, cx| {
                        cx.update_entity(&content_list, |list, cx| {
                            list.delegate_mut().select_all();
                            cx.notify();
                        })
                    }
                }),
        )
    }
}
