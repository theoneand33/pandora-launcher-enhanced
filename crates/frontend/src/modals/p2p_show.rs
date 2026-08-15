use std::{sync::Arc, time::Instant};

use bridge::{handle::BackendHandle, message::MessageToBackend};
use gpui::{ClipboardItem, prelude::*, *};
use gpui_component::{ActiveTheme, Sizable, WindowExt, button::Button, button::ButtonVariants, h_flex, v_flex};

use crate::icon::PandoraIcon;

#[derive(Clone, Copy, PartialEq, Eq)]
enum CopiedTarget {
    Primary,
    Row(usize),
}

struct P2pShowState {
    links: Arc<[Arc<str>]>,
    token: Arc<str>,
    expires_at_ms: i64,
    backend_handle: BackendHandle,
    copied: Option<(CopiedTarget, Instant)>,
}

impl P2pShowState {
    fn is_copied(&self, target: CopiedTarget) -> bool {
        match &self.copied {
            Some((t, at)) if *t == target && at.elapsed().as_secs_f32() < 2.0 => true,
            _ => false,
        }
    }

    fn clear_if_expired(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some((_, at)) = &self.copied {
            if at.elapsed().as_secs_f32() >= 2.0 {
                self.copied = None;
                cx.notify();
                return true;
            }
        }
        false
    }

    fn render(
        &mut self,
        dialog: gpui_component::dialog::Dialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_component::dialog::Dialog {
        // Auto-clear "Copied" after 2s without spawning a task — poll via animation frame.
        if self.copied.is_some() {
            if self.clear_if_expired(cx) {
                // cleared
            } else {
                window.request_animation_frame();
            }
        }

        let mut col = v_flex().gap_2();
        for (idx, link) in self.links.iter().enumerate() {
            let is_copied = self.is_copied(CopiedTarget::Row(idx));
            let label = if is_copied {
                t::instance::p2p::copied()
            } else {
                t::instance::p2p::copy()
            };
            let icon = if is_copied {
                PandoraIcon::Check
            } else {
                PandoraIcon::Copy
            };
            let link_text = link.to_string();
            col = col.child(h_flex().gap_2().child(div().flex_1().text_sm().child(link_text)).child(
                Button::new(("copy", idx)).label(label).icon(icon).small().on_click(cx.listener(
                    move |this, _, _, cx| {
                        let link = this.links[idx].clone();
                        cx.write_to_clipboard(ClipboardItem::new_string(link.to_string()));
                        this.copied = Some((CopiedTarget::Row(idx), Instant::now()));
                        cx.notify();
                    },
                )),
            ));
        }

        let remaining = ((self.expires_at_ms - chrono::Utc::now().timestamp_millis()) / 60000).max(0);
        col = col.child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(t::instance::p2p::expires(remaining)),
        );

        let token_short = self.token.chars().take(8).collect::<String>();
        let token_for_cancel = self.token.clone();
        let handle_for_cancel = self.backend_handle.clone();

        let primary_is_copied = self.is_copied(CopiedTarget::Primary);
        let primary_label = if primary_is_copied {
            t::instance::p2p::copied()
        } else {
            t::instance::p2p::copy_link()
        };
        let primary_icon = if primary_is_copied {
            PandoraIcon::Check
        } else {
            PandoraIcon::Copy
        };
        // Copy the first (primary) link — for relay mode this is the internet link,
        // for LAN mode the first LAN IP.
        let primary_link = self.links.first().cloned().unwrap_or_else(|| "".into());

        dialog
            .title(format!("{} ({token_short})", t::instance::p2p::share_title()))
            .child(col)
            .footer(
                h_flex()
                    .gap_2()
                    .child(Button::new("copy_link").label(primary_label).icon(primary_icon).on_click(cx.listener(
                        move |this, _, _, cx| {
                            let link = primary_link.clone();
                            if link.is_empty() {
                                return;
                            }
                            cx.write_to_clipboard(ClipboardItem::new_string(link.to_string()));
                            this.copied = Some((CopiedTarget::Primary, Instant::now()));
                            cx.notify();
                        },
                    )))
                    .child(Button::new("cancel_share").label(t::common::cancel()).danger().on_click({
                        let token = token_for_cancel.clone();
                        move |_, window, _cx| {
                            handle_for_cancel.send(MessageToBackend::CancelP2pShare { token: token.clone() });
                            window.close_dialog(_cx);
                        }
                    }))
                    .child(Button::new("ok").label(t::common::ok()).on_click(|_, window, cx| window.close_dialog(cx))),
            )
    }
}

pub fn open_p2p_show(
    links: Arc<[Arc<str>]>,
    token: Arc<str>,
    expires_at_ms: i64,
    backend_handle: BackendHandle,
    window: &mut Window,
    cx: &mut App,
) {
    let state = cx.new(|_cx| P2pShowState {
        links,
        token,
        expires_at_ms,
        backend_handle,
        copied: None,
    });
    window.open_dialog(cx, move |modal, window, cx| {
        cx.update_entity(&state, |state, cx| state.render(modal, window, cx))
    });
}
