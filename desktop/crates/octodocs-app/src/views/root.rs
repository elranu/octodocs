use std::cell::Cell;
use std::rc::Rc;

use adabraka_ui::prelude::*;
use gpui::Subscription;
use octodocs_core::FileIo;
use octodocs_github::SyncStatus;

use super::block_editor_pane::BlockEditorPane;
use super::editor_pane::EditorPane;
use super::github_panel::GitHubPanel;
use super::preview_pane::PreviewPane;
use crate::app_state::AppState;

pub struct RootView {
    app_state: Entity<AppState>,
    block_editor_pane: Entity<BlockEditorPane>,
    editor_pane: Entity<EditorPane>,
    preview_pane: Entity<PreviewPane>,
    github_panel: Entity<GitHubPanel>,
    toolbar: Entity<Toolbar>,
    _pane_subscription: Subscription,
    _github_panel_subscription: Subscription,
}

impl RootView {
    pub fn new(cx: &mut Context<Self>, initial_is_dark: bool) -> Self {
        let app_state = cx.new(|cx| AppState::new(cx));
        let block_editor_pane = cx.new(|_| BlockEditorPane::new(app_state.clone()));
        let editor_pane = cx.new(|_| EditorPane::new(app_state.clone()));
        let preview_pane = cx.new(|_| PreviewPane::new(app_state.clone()));
        let github_panel = cx.new(|_| GitHubPanel::new(app_state.clone()));

        // Re-render when AppState changes (content/block/mode changes).
        let pane_bep = block_editor_pane.clone();
        let pane_ep = editor_pane.clone();
        let pane_pp = preview_pane.clone();
        let subscription = cx.observe(&app_state, move |_, _, cx| {
            pane_bep.update(cx, |_, cx| cx.notify());
            pane_ep.update(cx, |_, cx| cx.notify());
            pane_pp.update(cx, |_, cx| cx.notify());
        });

        // Also re-render root when AppState changes (for github_panel_open).
        let github_panel_clone = github_panel.clone();
        let github_panel_subscription = cx.observe(&app_state, move |_this, _, cx| {
            cx.notify();
            github_panel_clone.update(cx, |_, cx| cx.notify());
        });

        // editor_weak targets the shared block editor — toolbar actions operate
        // on whichever block is currently active.
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

        let github_panel_weak = github_panel.downgrade();
        let aw_github = app_weak.clone();
        let github_h = move |_w: &mut Window, cx: &mut App| {
            let _ = aw_github.update(cx, |state, cx| {
                state.github_panel_open = !state.github_panel_open;
                cx.notify();
            });
            // Initialize the panel when opening
            let _ = github_panel_weak.update(cx, |panel, cx| panel.init(cx));
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
                        )
                        .separator()
                        .button(
                            ToolbarButton::new("github", IconSource::Named("github".into()))
                                .tooltip("GitHub Sync")
                                .on_click(github_h),
                        ),
                )
        });

        Self {
            app_state,
            block_editor_pane,
            editor_pane,
            preview_pane,
            github_panel,
            toolbar,
            _pane_subscription: subscription,
            _github_panel_subscription: github_panel_subscription,
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
        let view_mode = app.view_mode;
        let github_panel_open = app.github_panel_open;
        let github_sync_status = app.github_sync_status.clone();
        drop(app);

        let dirty_dot = if dirty { "● " } else { "" };

        let app_weak = self.app_state.downgrade();
        let mode_badge = div()
            .flex()
            .items_center()
            .px(px(8.0))
            .py(px(2.0))
            .rounded(px(4.0))
            .bg(theme.tokens.muted)
            .cursor_pointer()
            .hover(|s| s.bg(theme.tokens.accent))
            .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                let _ = app_weak.update(cx, |state, cx| state.cycle_view_mode(cx));
            })
            .child(body_small(view_mode.label()));

        // Sync status badge for status bar (clickable to open GitHub panel)
        let github_panel_weak = self.github_panel.downgrade();
        let app_weak_sync = self.app_state.downgrade();
        let sync_badge_content: AnyElement = match &github_sync_status {
            SyncStatus::Idle => div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(Icon::new(IconSource::Named("cloud-off".into())).size_3().color(theme.tokens.muted_foreground))
                .into_any_element(),
            SyncStatus::Syncing => div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(Spinner::new().size(SpinnerSize::Xs))
                .child(body_small("Syncing...").color(theme.tokens.muted_foreground))
                .into_any_element(),
            SyncStatus::Success { .. } => div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(Icon::new(IconSource::Named("check".into())).size_3().color(theme.tokens.primary))
                .child(body_small("Synced").color(theme.tokens.primary))
                .into_any_element(),
            SyncStatus::Failed { message: _ } => div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(Icon::new(IconSource::Named("alert-circle".into())).size_3().color(theme.tokens.destructive))
                .child(body_small("Sync failed").color(theme.tokens.destructive))
                .into_any_element(),
        };

        let sync_badge = div()
            .id("sync-badge")
            .cursor_pointer()
            .px(px(6.0))
            .py(px(2.0))
            .rounded(px(4.0))
            .hover(|s| s.bg(theme.tokens.accent))
            .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                let _ = app_weak_sync.update(cx, |state, cx| {
                    state.github_panel_open = !state.github_panel_open;
                    cx.notify();
                });
                let _ = github_panel_weak.update(cx, |panel, cx| panel.init(cx));
            })
            .child(sync_badge_content);

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
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(sync_badge)
                    .child(mode_badge)
                    .child(body_small("UTF-8")),
            );

        // Build content area based on view mode.
        let content_area: AnyElement = match view_mode {
            crate::app_state::ViewMode::Wysiwyg => div()
                .flex()
                .flex_grow()
                .min_h_0()
                .child(self.block_editor_pane.clone())
                .into_any_element(),
            crate::app_state::ViewMode::Source => div()
                .flex()
                .flex_grow()
                .min_h_0()
                .child(self.editor_pane.clone())
                .into_any_element(),
            crate::app_state::ViewMode::Split => div()
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
                )
                .into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.tokens.background)
            .child(self.toolbar.clone())
            .child(content_area)
            .child(status_bar)
            .when(github_panel_open, |this| {
                this.child(self.github_panel.clone())
            })
    }
}

