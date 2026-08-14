use bridge::{
    handle::BackendHandle,
    instance::InstanceID,
    message::{ExportCurseforgeOptions, ExportModrinthOptions, ExportOptions, MessageToBackend},
    modal_action::ModalAction,
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, WindowExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex, v_flex,
};

use crate::modals::generic;

struct P2pShareState {
    instance_id: InstanceID,
    backend_handle: BackendHandle,
    include_saves: bool,
    include_mods: bool,
    include_resourcepacks: bool,
    include_shaders: bool,
    include_configs: bool,
    include_screenshots: bool,
    include_backups: bool,
    include_logs: bool,
    include_cache: bool,
    include_synced: bool,
    use_relay: bool,
}

impl P2pShareState {
    fn build_options(&self) -> ExportOptions {
        ExportOptions {
            include_saves: self.include_saves,
            include_mods: self.include_mods,
            include_resourcepacks: self.include_resourcepacks,
            include_shaders: self.include_shaders,
            include_configs: self.include_configs,
            include_screenshots: self.include_screenshots,
            include_backups: self.include_backups,
            include_logs: self.include_logs,
            include_cache: self.include_cache,
            include_synced: self.include_synced,
            modrinth: ExportModrinthOptions {
                name: "".into(),
                version: "1.0.0".into(),
                summary: None,
            },
            curseforge: ExportCurseforgeOptions {
                name: "".into(),
                version: "1.0.0".into(),
                author: None,
                recommended_ram: None,
            },
        }
    }

    fn render(
        &mut self,
        dialog: gpui_component::dialog::Dialog,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_component::dialog::Dialog {
        let options = v_flex()
            .gap_2()
            .child(
                Checkbox::new("p2p_saves")
                    .checked(self.include_saves)
                    .label(t::instance::export::include_saves())
                    .on_click(cx.listener(|this, v, _, cx| {
                        this.include_saves = *v;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("p2p_mods")
                    .checked(self.include_mods)
                    .label(t::instance::export::include_mods())
                    .on_click(cx.listener(|this, v, _, cx| {
                        this.include_mods = *v;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("p2p_res")
                    .checked(self.include_resourcepacks)
                    .label(t::instance::export::include_resourcepacks())
                    .on_click(cx.listener(|this, v, _, cx| {
                        this.include_resourcepacks = *v;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("p2p_shaders")
                    .checked(self.include_shaders)
                    .label(t::instance::export::include_shaders())
                    .on_click(cx.listener(|this, v, _, cx| {
                        this.include_shaders = *v;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("p2p_configs")
                    .checked(self.include_configs)
                    .label(t::instance::export::include_configs())
                    .on_click(cx.listener(|this, v, _, cx| {
                        this.include_configs = *v;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("p2p_screens")
                    .checked(self.include_screenshots)
                    .label(t::instance::export::include_screenshots())
                    .on_click(cx.listener(|this, v, _, cx| {
                        this.include_screenshots = *v;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("p2p_backups")
                    .checked(self.include_backups)
                    .label(t::instance::export::include_backups())
                    .on_click(cx.listener(|this, v, _, cx| {
                        this.include_backups = *v;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("p2p_logs")
                    .checked(self.include_logs)
                    .label(t::instance::export::include_logs())
                    .on_click(cx.listener(|this, v, _, cx| {
                        this.include_logs = *v;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("p2p_cache")
                    .checked(self.include_cache)
                    .label(t::instance::export::include_cache())
                    .on_click(cx.listener(|this, v, _, cx| {
                        this.include_cache = *v;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("p2p_synced")
                    .checked(self.include_synced)
                    .label(t::instance::export::include_synced())
                    .on_click(cx.listener(|this, v, _, cx| {
                        this.include_synced = *v;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("p2p_use_relay")
                    .checked(self.use_relay)
                    .label(t::instance::p2p::use_relay())
                    .on_click(cx.listener(|this, v, _, cx| {
                        this.use_relay = *v;
                        cx.notify();
                    })),
            );

        let hint = div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(t::instance::p2p::relay_hint());

        dialog
            .title(t::instance::p2p::share_title())
            .child(
                v_flex()
                    .gap_3()
                    .child(crate::labelled(t::instance::export::options(), options))
                    .child(hint),
            )
            .footer(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("cancel")
                            .label(t::common::cancel())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(Button::new("share").label(t::instance::p2p::share_action()).success().on_click({
                        let id = self.instance_id;
                        let handle = self.backend_handle.clone();
                        let options = self.build_options();
                        let use_relay = self.use_relay;
                        move |_, window, cx| {
                            window.close_dialog(cx);
                            let modal = ModalAction::default();
                            generic::show_modal(
                                window,
                                cx,
                                t::instance::p2p::progress().into(),
                                t::instance::p2p::error().into(),
                                modal.clone(),
                            );
                            handle.send(MessageToBackend::CreateP2pShare {
                                id,
                                options: options.clone(),
                                modal_action: modal,
                                use_relay,
                            });
                        }
                    })),
            )
    }
}

pub fn open_p2p_share(instance_id: InstanceID, backend_handle: BackendHandle, window: &mut Window, cx: &mut App) {
    let state = cx.new(|_cx| P2pShareState {
        instance_id,
        backend_handle,
        include_saves: false,
        include_mods: true,
        include_resourcepacks: false,
        include_shaders: false,
        include_configs: true,
        include_screenshots: false,
        include_backups: false,
        include_logs: false,
        include_cache: false,
        include_synced: false,
        use_relay: false,
    });
    window.open_dialog(cx, move |modal, window, cx| {
        cx.update_entity(&state, |state, cx| state.render(modal, window, cx))
    });
}
