//! Insert Link Modal — dialog to insert (or edit) a hyperlink span.

use adabraka_ui::components::confirm_dialog::Dialog as ModalDialog;
use adabraka_ui::components::input::Input;
use adabraka_ui::components::input_state::InputState;
use adabraka_ui::prelude::*;
use gpui::Subscription;

use crate::app_state::AppState;

pub struct InsertLinkModal {
    app_state: Entity<AppState>,
    text_input: Entity<InputState>,
    url_input: Entity<InputState>,
    /// Track whether the modal was open on the last render cycle so we can
    /// detect the open transition and pre-fill the text field.
    was_open: bool,
    _sub: Subscription,
}

impl InsertLinkModal {
    pub fn new(app_state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let text_input = cx.new(|cx| InputState::new(cx).placeholder("Link text"));
        let url_input = cx.new(|cx| InputState::new(cx).placeholder("https://example.com"));

        // Observe app_state to detect when modal is opened and pre-fill inputs.
        let text_weak = text_input.downgrade();
        let url_weak = url_input.downgrade();
        let sub = cx.observe(&app_state, move |this, _, cx| {
            let (now_open, prefill) = {
                let app = this.app_state.read(cx);
                (app.insert_link_modal_open, app.insert_link_prefill_text.clone())
            };
            if now_open && !this.was_open {
                // Just opened — pre-fill text from selection, clear URL field.
                let _ = text_weak.update(cx, |input, _| {
                    input.content = prefill.into();
                });
                let _ = url_weak.update(cx, |input, _| {
                    input.content = "".into();
                });
            }
            this.was_open = now_open;
            cx.notify();
        });

        Self {
            app_state,
            text_input,
            url_input,
            was_open: false,
            _sub: sub,
        }
    }

    fn close(&self, cx: &mut Context<Self>) {
        self.app_state.update(cx, |state, cx| {
            state.insert_link_modal_open = false;
            cx.notify();
        });
    }

    fn insert(&self, cx: &mut Context<Self>) {
        let text = self.text_input.read(cx).content().to_string();
        let url = self.url_input.read(cx).content().to_string();

        if url.is_empty() {
            // Nothing to insert without a URL — just close.
            self.close(cx);
            return;
        }

        let insert_text = if text.is_empty() { url.clone() } else { text };

        self.app_state.update(cx, |state, cx| {
            state.doc_editor.update(cx, |editor, cx| {
                editor.insert_link(insert_text, url, cx);
            });
            state.insert_link_modal_open = false;
            cx.notify();
        });
    }
}

impl Render for InsertLinkModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = use_theme();
        let weak = cx.entity().downgrade();

        let close_weak = weak.clone();
        let close_handler = move |_w: &mut Window, cx: &mut App| {
            let _ = close_weak.update(cx, |modal, cx| modal.close(cx));
        };

        // ── Header ────────────────────────────────────────────────────────
        let close_icon_weak = weak.clone();
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .p(px(16.0))
            .border_b_1()
            .border_color(theme.tokens.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(Icon::new(IconSource::Named("link".into())).size_4())
                    .child(h4("Insert Link")),
            )
            .child(
                IconButton::new(IconSource::Named("x".into()))
                    .size(px(28.0))
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _w, cx| {
                        let _ = close_icon_weak.update(cx, |modal, cx| modal.close(cx));
                    }),
            );

        // ── Content ───────────────────────────────────────────────────────
        let content = div()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            // Text field
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(label("Text"))
                    .child(Input::new(&self.text_input)),
            )
            // URL field
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(label("URL"))
                    .child(Input::new(&self.url_input)),
            );

        // ── Footer / action buttons ────────────────────────────────────────
        let cancel_weak = weak.clone();
        let insert_weak = weak.clone();
        let footer = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(8.0))
            .p(px(16.0))
            .border_t_1()
            .border_color(theme.tokens.border)
            .child(
                Button::new("cancel-link", "Cancel")
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _w, cx| {
                        let _ = cancel_weak.update(cx, |modal, cx| modal.close(cx));
                    }),
            )
            .child(
                Button::new("insert-link", "Insert")
                    .variant(ButtonVariant::Default)
                    .on_click(move |_, _w, cx| {
                        let _ = insert_weak.update(cx, |modal, cx| modal.insert(cx));
                    }),
            );

        ModalDialog::new()
            .width(px(420.0))
            .on_backdrop_click(close_handler)
            .header(header)
            .content(content)
            .footer(footer)
    }
}
