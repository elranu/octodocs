use std::cell::Cell;
use std::rc::Rc;

use adabraka_ui::prelude::*;
use gpui::Subscription;
use octodocs_core::FileIo;

use super::editor_pane::EditorPane;
use super::preview_pane::PreviewPane;
use crate::app_state::AppState;

pub struct RootView {
    app_state: Entity<AppState>,
    editor_pane: Entity<EditorPane>,
    preview_pane: Entity<PreviewPane>,
    toolbar: Entity<Toolbar>,
    _preview_subscription: Subscription,
}

impl RootView {
    pub fn new(cx: &mut Context<Self>, initial_is_dark: bool) -> Self {
        let app_state = cx.new(|cx| AppState::new(cx));
        let editor_pane = cx.new(|_| EditorPane::new(app_state.clone()));
        let preview_pane = cx.new(|_| PreviewPane::new(app_state.clone()));

        let preview = preview_pane.clone();
        let subscription = cx.observe(&app_state, move |_, _, cx| {
            preview.update(cx, |_, cx| cx.notify());
        });

        let editor_weak = app_state.read(cx).editor_state.downgrade();
        let app_weak = app_state.downgrade();

        let aw = app_weak.clone();
        let new_h = move |_w: &mut Window, cx: &mut App| {
            let _ = aw.update(cx, |state, cx| state.new_document(cx));
        };

        let aw = app_weak.clone();
        let open_h = move |_w: &mut Window, cx: &mut App| {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Markdown", &["md", "markdown"])
                .pick_file()
            {
                match FileIo::open(&path) {
                    Ok(doc) => {
                        let _ = aw.update(cx, |state, cx| state.load_document(doc, cx));
                    }
                    Err(e) => eprintln!("Open error: {e}"),
                }
            }
        };

        let aw = app_weak.clone();
        let save_h = move |_w: &mut Window, cx: &mut App| {
            let _ = aw.update(cx, |state, cx| state.save(cx));
        };

        let aw = app_weak.clone();
        let save_as_h = move |_w: &mut Window, cx: &mut App| {
            let _ = aw.update(cx, |state, cx| state.save_as(cx));
        };

        let ew = editor_weak.clone();
        let bold_h = move |_w: &mut Window, cx: &mut App| {
            let _ = ew.update(cx, |state, cx| state.wrap_selection("**", "**", cx));
        };

        let ew = editor_weak.clone();
        let italic_h = move |_w: &mut Window, cx: &mut App| {
            let _ = ew.update(cx, |state, cx| state.wrap_selection("*", "*", cx));
        };

        let ew = editor_weak.clone();
        let code_h = move |_w: &mut Window, cx: &mut App| {
            let _ = ew.update(cx, |state, cx| state.wrap_selection("`", "`", cx));
        };

        let ew = editor_weak.clone();
        let h1_h = move |_w: &mut Window, cx: &mut App| {
            let _ = ew.update(cx, |state, cx| state.insert_text("# ", cx));
        };

        let ew = editor_weak.clone();
        let h2_h = move |_w: &mut Window, cx: &mut App| {
            let _ = ew.update(cx, |state, cx| state.insert_text("## ", cx));
        };

        let is_dark = Rc::new(Cell::new(initial_is_dark));
        let is_dark_toggle = is_dark.clone();
        let theme_h = move |_w: &mut Window, cx: &mut App| {
            if is_dark_toggle.get() {
                install_theme(cx, Theme::light());
            } else {
                install_theme(cx, Theme::dark());
            }
            is_dark_toggle.set(!is_dark_toggle.get());
        };

        let toolbar = cx.new(|_| {
            Toolbar::new()
                .size(ToolbarSize::Md)
                .group(
                    ToolbarGroup::new()
                        .button(
                            ToolbarButton::new("new", IconSource::Named("file-plus".into()))
                                .tooltip("New (Ctrl+N)")
                                .on_click(new_h),
                        )
                        .button(
                            ToolbarButton::new("open", IconSource::Named("folder-open".into()))
                                .tooltip("Open… (Ctrl+O)")
                                .on_click(open_h),
                        )
                        .button(
                            ToolbarButton::new("save", IconSource::Named("save".into()))
                                .tooltip("Save (Ctrl+S)")
                                .on_click(save_h),
                        )
                        .button(
                            ToolbarButton::new("save-as", IconSource::Named("save-all".into()))
                                .tooltip("Save As…")
                                .on_click(save_as_h),
                        ),
                )
                .group(
                    ToolbarGroup::new()
                        .button(
                            ToolbarButton::new("bold", IconSource::Named("bold".into()))
                                .tooltip("Bold (Ctrl+B)")
                                .on_click(bold_h),
                        )
                        .button(
                            ToolbarButton::new("italic", IconSource::Named("italic".into()))
                                .tooltip("Italic (Ctrl+I)")
                                .on_click(italic_h),
                        )
                        .separator()
                        .button(
                            ToolbarButton::new("h1", IconSource::Named("heading-1".into()))
                                .tooltip("Heading 1")
                                .on_click(h1_h),
                        )
                        .button(
                            ToolbarButton::new("h2", IconSource::Named("heading-2".into()))
                                .tooltip("Heading 2")
                                .on_click(h2_h),
                        )
                        .button(
                            ToolbarButton::new("code", IconSource::Named("code".into()))
                                .tooltip("Inline Code")
                                .on_click(code_h),
                        )
                        .separator()
                        .button(
                            ToolbarButton::new("theme", IconSource::Named("moon".into()))
                                .tooltip("Toggle Theme")
                                .on_click(theme_h),
                        ),
                )
        });

        Self {
            app_state,
            editor_pane,
            preview_pane,
            toolbar,
            _preview_subscription: subscription,
        }
    }
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = use_theme();
        let app = self.app_state.read(cx);
        let title = app.document.title();
        let word_count = app.document.word_count();
        let dirty = app.dirty;

        let dirty_dot = if dirty { "● " } else { "" };

        let status_bar = div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(28.0))
            .px(px(12.0))
            .bg(theme.tokens.card)
            .border_t_1()
            .border_color(theme.tokens.border)
            .child(body_small(format!("{}{}", dirty_dot, title)))
            .child(body_small(format!("{} words", word_count)))
            .child(body_small("UTF-8"));

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.tokens.background)
            .child(self.toolbar.clone())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_grow()
                    .min_h_0()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w_1_2()
                            .h_full()
                            .border_r_1()
                            .border_color(theme.tokens.border)
                            .child(self.editor_pane.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w_1_2()
                            .h_full()
                            .child(self.preview_pane.clone()),
                    ),
            )
            .child(status_bar)
    }
}