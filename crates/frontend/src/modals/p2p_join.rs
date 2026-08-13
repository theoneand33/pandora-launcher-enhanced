use bridge::{handle::BackendHandle, message::MessageToBackend, modal_action::ModalAction};
use gpui::{prelude::*, *};
use gpui_component::{
    WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    v_flex,
};

use crate::modals::generic;

struct P2pJoinState {
    backend_handle: BackendHandle,
    link_input: Entity<InputState>,
    name_input: Entity<InputState>,
}

impl P2pJoinState {
    fn render(
        &mut self,
        dialog: gpui_component::dialog::Dialog,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> gpui_component::dialog::Dialog {
        dialog
            .title(t::instance::p2p::join_title())
            .child(
                v_flex()
                    .gap_3()
                    .child(crate::labelled(t::instance::p2p::link(), Input::new(&self.link_input)))
                    .child(crate::labelled(t::instance::p2p::target_name(), Input::new(&self.name_input))),
            )
            .footer(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("cancel")
                            .label(t::common::cancel())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(Button::new("join").label(t::instance::p2p::join_action()).success().on_click({
                        let handle = self.backend_handle.clone();
                        let link_input = self.link_input.clone();
                        let name_input = self.name_input.clone();
                        move |_, window, cx| {
                            let link = link_input.read(cx).value().trim().to_string();
                            if link.is_empty() {
                                return;
                            }
                            // allow bare token → assume relay url is set; still try as full url
                            let target = {
                                let v = name_input.read(cx).value().trim().to_string();
                                if v.is_empty() { None } else { Some(v) }
                            };
                            window.close_dialog(cx);
                            let modal = ModalAction::default();
                            generic::show_modal(
                                window,
                                cx,
                                t::instance::p2p::progress().into(),
                                t::instance::p2p::error().into(),
                                modal.clone(),
                            );
                            // if bare token without scheme, try to prepend relay if configured – backend will also accept token-only
                            handle.send(MessageToBackend::JoinP2pShare {
                                link,
                                target_name: target,
                                modal_action: modal,
                            });
                        }
                    })),
            )
    }
}

pub fn open_p2p_join(backend_handle: BackendHandle, window: &mut Window, cx: &mut App) {
    let link_input =
        cx.new(|cx| InputState::new(window, cx).placeholder("https://relay.example.com/p2p/<token> or token"));
    let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("p2p-import"));
    let state = cx.new(|_cx| P2pJoinState {
        backend_handle,
        link_input,
        name_input,
    });
    window.open_dialog(cx, move |modal, window, cx| {
        cx.update_entity(&state, |state, cx| state.render(modal, window, cx))
    });
}
