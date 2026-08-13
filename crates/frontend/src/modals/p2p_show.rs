use gpui::{prelude::*, *};
use gpui_component::{Sizable, WindowExt, button::Button, h_flex, v_flex};
use std::sync::Arc;

pub fn open_p2p_show(links: Arc<[Arc<str>]>, token: Arc<str>, window: &mut Window, cx: &mut App) {
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
        let token_short = token.chars().take(8).collect::<String>();
        modal
            .title(format!("{} ({token_short})", t::instance::p2p::share_title()))
            .child(col)
            .footer(Button::new("ok").label(t::common::ok()).on_click(|_, window, cx| window.close_dialog(cx)))
    });
}
