use bridge::{handle::BackendHandle, message::MessageToBackend};
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Sizable, WindowExt, button::Button, button::ButtonVariants, h_flex, v_flex};
use std::sync::Arc;

pub fn open_p2p_show(
    links: Arc<[Arc<str>]>,
    token: Arc<str>,
    expires_at_ms: i64,
    backend_handle: BackendHandle,
    window: &mut Window,
    cx: &mut App,
) {
    window.open_dialog(cx, move |modal, _window, _cx| {
        let mut col = v_flex().gap_2();
        for (idx, link) in links.iter().enumerate() {
            let link_clone = link.clone();
            let link_text = link_clone.clone();
            col = col.child(h_flex().gap_2().child(div().flex_1().text_sm().child(link_text.to_string())).child(
                Button::new(("copy", idx)).label(t::instance::p2p::copy()).small().on_click({
                    let link = link_clone.clone();
                    move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(link.to_string()));
                    }
                }),
            ));
        }
        let remaining = ((expires_at_ms - chrono::Utc::now().timestamp_millis()) / 60000).max(0);
        col = col.child(
            div()
                .text_sm()
                .text_color(_cx.theme().muted_foreground)
                .child(format!("Expires in {remaining} min — keep launcher open")),
        );
        let token_short = token.chars().take(8).collect::<String>();
        let token_for_cancel = token.clone();
        let handle_for_cancel = backend_handle.clone();
        modal
            .title(format!("{} ({token_short})", t::instance::p2p::share_title()))
            .child(col)
            .footer(
                h_flex()
                    .gap_2()
                    .child(Button::new("cancel_share").label(t::common::cancel()).danger().on_click({
                        let token = token_for_cancel.clone();
                        move |_, window, _cx| {
                            handle_for_cancel.send(MessageToBackend::CancelP2pShare { token: token.clone() });
                            window.close_dialog(_cx);
                        }
                    }))
                    .child(Button::new("ok").label(t::common::ok()).on_click(|_, window, cx| window.close_dialog(cx))),
            )
    });
}
