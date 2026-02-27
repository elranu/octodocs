use std::cell::Cell;
use std::rc::Rc;

use adabraka_ui::components::confirm_dialog::Dialog as ModalDialog;
use adabraka_ui::prelude::*;
use gpui::Subscription;
use octodocs_core::{FileIo, ParagraphKind};
use octodocs_github::{get_stored_token, SyncStatus};

use super::document_editor_pane::DocumentEditorPane;
use super::editor_pane::EditorPane;
use super::github_auth_modal::GithubAuthModal;
use super::github_sidebar::GithubSidebar;
use super::preview_pane::PreviewPane;
use super::repo_add_modal::RepoAddModal;
use crate::app_state::{AppState, PostAuthAction};

pub struct RootView {
    pub app_state: Entity<AppState>,
    document_editor_pane: Entity<DocumentEditorPane>,
    editor_pane: Entity<EditorPane>,
    preview_pane: Entity<PreviewPane>,
    github_sidebar: Entity<GithubSidebar>,
    github_auth_modal: Entity<GithubAuthModal>,
    repo_add_modal: Entity<RepoAddModal>,
    toolbar: Entity<Toolbar>,
    _pane_subscription: Subscription,
    _root_subscription: Subscription,
}

impl RootView {
    pub fn new(cx: &mut Context<Self>, initial_is_dark: bool) -> Self {
        let app_state = cx.new(AppState::new);
        let document_editor_pane = cx.new(|_| DocumentEditorPane::new(app_state.clone()));
        let editor_pane = cx.new(|_| EditorPane::new(app_state.clone()));
        let preview_pane = cx.new(|_| PreviewPane::new(app_state.clone()));
        let github_sidebar = cx.new(|cx| GithubSidebar::new(app_state.clone(), cx));
        let repo_add_modal = cx.new(|cx| RepoAddModal::new(app_state.clone(), cx));
        let repo_modal_weak_for_auth = repo_add_modal.downgrade();
        let github_auth_modal = cx.new(|_| {
            GithubAuthModal::new(
                app_state.clone(),
                Box::new(move |token, cx| {
                    let _ = repo_modal_weak_for_auth.update(cx, |modal, cx| {
                        modal.set_auth_token(token, cx);
                    });
                }),
            )
        });

        // Re-render when AppState changes (content/block/mode changes).
        let pane_dep = document_editor_pane.clone();
        let pane_ep = editor_pane.clone();
        let pane_pp = preview_pane.clone();
        let pane_sb = github_sidebar.clone();
        let subscription = cx.observe(&app_state, move |_, _, cx| {
            pane_dep.update(cx, |_, cx| cx.notify());
            pane_ep.update(cx, |_, cx| cx.notify());
            pane_pp.update(cx, |_, cx| cx.notify());
            pane_sb.update(cx, |_, cx| cx.notify());
        });

        // Re-render root and overlays when AppState changes.
        let auth_modal_clone = github_auth_modal.clone();
        let repo_modal_clone = repo_add_modal.clone();
        let root_subscription = cx.observe(&app_state, move |_this, _, cx| {
            cx.notify();
            auth_modal_clone.update(cx, |_, cx| cx.notify());
            repo_modal_clone.update(cx, |_, cx| cx.notify());
        });

        // toolbar actions operate on the single doc_editor entity.
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

        let aw = app_weak.clone();
        let bold_h = move |_w: &mut Window, cx: &mut App| {
            let _ = aw.update(cx, |state, cx| {
                state.doc_editor.update(cx, |editor, cx| editor.toggle_bold(cx));
            });
        };

        let aw = app_weak.clone();
        let italic_h = move |_w: &mut Window, cx: &mut App| {
            let _ = aw.update(cx, |state, cx| {
                state.doc_editor.update(cx, |editor, cx| editor.toggle_italic(cx));
            });
        };

        let aw = app_weak.clone();
        let code_h = move |_w: &mut Window, cx: &mut App| {
            let _ = aw.update(cx, |state, cx| {
                state.doc_editor.update(cx, |editor, cx| editor.toggle_code(cx));
            });
        };

        let aw = app_weak.clone();
        let underline_h = move |_w: &mut Window, cx: &mut App| {
            let _ = aw.update(cx, |state, cx| {
                state.doc_editor.update(cx, |editor, cx| editor.toggle_underline(cx));
            });
        };

        let aw = app_weak.clone();
        let strike_h = move |_w: &mut Window, cx: &mut App| {
            let _ = aw.update(cx, |state, cx| {
                state.doc_editor.update(cx, |editor, cx| editor.toggle_strikethrough(cx));
            });
        };

        let aw = app_weak.clone();
        let h1_h = move |_w: &mut Window, cx: &mut App| {
            let _ = aw.update(cx, |state, cx| {
                state.doc_editor.update(cx, |editor, cx| {
                    editor.set_paragraph_kind(ParagraphKind::Heading(1), cx);
                });
            });
        };

        let aw = app_weak.clone();
        let h2_h = move |_w: &mut Window, cx: &mut App| {
            let _ = aw.update(cx, |state, cx| {
                state.doc_editor.update(cx, |editor, cx| {
                    editor.set_paragraph_kind(ParagraphKind::Heading(2), cx);
                });
            });
        };

        let aw = app_weak.clone();
        let insert_table_h = move |_w: &mut Window, cx: &mut App| {
            let _ = aw.update(cx, |state, cx| {
                state.doc_editor.update(cx, |editor, cx| editor.insert_table(cx));
            });
        };

        let aw = app_weak.clone();
        let add_row_h = move |_w: &mut Window, cx: &mut App| {
            let _ = aw.update(cx, |state, cx| {
                state.doc_editor.update(cx, |editor, cx| editor.add_table_row(cx));
            });
        };

        let aw = app_weak.clone();
        let remove_row_h = move |_w: &mut Window, cx: &mut App| {
            let _ = aw.update(cx, |state, cx| {
                state.doc_editor.update(cx, |editor, cx| editor.remove_table_row(cx));
            });
        };

        let aw = app_weak.clone();
        let add_col_h = move |_w: &mut Window, cx: &mut App| {
            let _ = aw.update(cx, |state, cx| {
                state.doc_editor.update(cx, |editor, cx| editor.add_table_column(cx));
            });
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

        let github_auth_modal_weak = github_auth_modal.downgrade();
        let aw_github = app_weak.clone();
        let github_h = move |_w: &mut Window, cx: &mut App| {
            match get_stored_token() {
                Ok(Some(_)) => {
                    let _ = aw_github.update(cx, |state, cx| {
                        state.repo_add_modal_open = true;
                        state.auth_modal_open = false;
                        cx.notify();
                    });
                }
                _ => {
                    let _ = aw_github.update(cx, |state, cx| {
                        state.auth_modal_open = true;
                        cx.notify();
                    });
                    let _ = github_auth_modal_weak.update(cx, |modal, cx| modal.init(cx));
                }
            }
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
                        .button(
                            ToolbarButton::new("underline", IconSource::Named("underline".into()))
                                .tooltip("Underline")
                                .on_click(underline_h),
                        )
                        .button(
                            ToolbarButton::new("strikethrough", IconSource::Named("strikethrough".into()))
                                .tooltip("Strikethrough")
                                .on_click(strike_h),
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
                        .button(
                            ToolbarButton::new("table", IconSource::Named("table".into()))
                                .tooltip("Insert Table")
                                .on_click(insert_table_h),
                        )
                        .button(
                            ToolbarButton::new("row-add", IconSource::Named("row-add".into()))
                                .tooltip("Add Row (Tab at end)")
                                .on_click(add_row_h),
                        )
                        .button(
                            ToolbarButton::new("col-add", IconSource::Named("col-add".into()))
                                .tooltip("Add Column")
                                .on_click(add_col_h),
                        )
                        .button(
                            ToolbarButton::new("row-remove", IconSource::Named("x".into()))
                                .tooltip("Remove Row")
                                .on_click(remove_row_h),
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
                                .tooltip("GitHub Authentication")
                                .on_click(github_h),
                        ),
                )
        });

        let should_force_onboarding = app_state.read(cx).github_bindings.is_empty();
        if should_force_onboarding {
            match get_stored_token() {
                Ok(Some(_)) => {
                    app_state.update(cx, |state, cx| {
                        state.repo_add_modal_open = true;
                        state.auth_modal_open = false;
                        state.sidebar_open = true;
                        cx.notify();
                    });
                }
                _ => {
                    app_state.update(cx, |state, cx| {
                        state.pending_post_auth_action = Some(PostAuthAction::AddRepo);
                        state.auth_modal_open = true;
                        state.repo_add_modal_open = false;
                        state.sidebar_open = true;
                        cx.notify();
                    });
                    github_auth_modal.update(cx, |modal, cx| modal.init(cx));
                }
            }
        }

        Self {
            app_state,
            document_editor_pane,
            editor_pane,
            preview_pane,
            github_sidebar,
            github_auth_modal,
            repo_add_modal,
            toolbar,
            _pane_subscription: subscription,
            _root_subscription: root_subscription,
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
        let auth_modal_open = app.auth_modal_open;
        let repo_add_modal_open = app.repo_add_modal_open;
        let sidebar_open = app.sidebar_open;
        let show_unsaved_prompt = app.show_unsaved_prompt;
        let pending_window_close = app.pending_window_close;
        let github_sync_status = app.github_sync_status.clone();
        let github_sync_configured = !app.github_bindings.is_empty();
        let current_doc_path = app.document.path.clone();
        let has_unsynced_changes = app.dirty && app.document.path.is_some();
        let last_import_summary = app.last_import_summary.clone();
        let current_file_synced = match (app.document.path.as_ref(), app.last_synced_path.as_ref()) {
            (Some(current), Some(last)) => current == last,
            _ => false,
        };
        let _ = app;

        if repo_add_modal_open {
            if let Ok(Some(token)) = get_stored_token() {
                self.repo_add_modal.update(cx, |modal, cx| {
                    modal.set_auth_token(token, cx);
                });
            }
        }

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

        // Sync status badge for status bar (clickable to open auth modal)
        let github_auth_modal_weak = self.github_auth_modal.downgrade();
        let app_weak_sync = self.app_state.downgrade();
        let sync_badge_content: AnyElement = match &github_sync_status {
            SyncStatus::Idle => div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(
                    if !github_sync_configured {
                        Icon::new(IconSource::Named("cloud-off".into())).size_3().color(theme.tokens.muted_foreground)
                    } else if has_unsynced_changes {
                        Icon::new(IconSource::Named("alert-circle".into())).size_3().color(theme.tokens.destructive)
                    } else if current_file_synced {
                        Icon::new(IconSource::Named("cloud".into())).size_3().color(theme.tokens.primary)
                    } else {
                        Icon::new(IconSource::Named("cloud".into())).size_3().color(theme.tokens.muted_foreground)
                    }
                )
                .child(
                    if !github_sync_configured {
                        body_small("Not configured").color(theme.tokens.muted_foreground)
                    } else if has_unsynced_changes {
                        body_small("Unsynced changes").color(theme.tokens.destructive)
                    } else if current_file_synced {
                        body_small("Synced").color(theme.tokens.primary)
                    } else {
                        body_small("Ready to sync").color(theme.tokens.muted_foreground)
                    }
                )
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
                .child(
                    if current_file_synced {
                        body_small("Synced").color(theme.tokens.primary)
                    } else if current_doc_path.is_some() {
                        body_small("Synced (other file)").color(theme.tokens.muted_foreground)
                    } else {
                        body_small("Synced").color(theme.tokens.primary)
                    }
                )
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
                    state.auth_modal_open = true;
                    cx.notify();
                });
                let _ = github_auth_modal_weak.update(cx, |modal, cx| modal.init(cx));
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
                    .when_some(last_import_summary, |this, summary| {
                        this.child(body_small(summary).color(theme.tokens.muted_foreground))
                    })
                    .child(sync_badge)
                    .child(mode_badge)
                    .child(body_small("UTF-8")),
            );

        // Build editor area based on view mode.
        let editor_area: AnyElement = match view_mode {
            crate::app_state::ViewMode::Wysiwyg => div()
                .flex()
                .flex_grow()
                .min_h_0()
                .child(self.document_editor_pane.clone())
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

        let content_area = div()
            .flex()
            .flex_row()
            .flex_grow()
            .min_h_0()
            .when(sidebar_open, |this| this.child(self.github_sidebar.clone()))
            .when(!sidebar_open, |this| {
                let app_weak_toggle_collapsed = self.app_state.downgrade();
                this.child(
                    div()
                        .w(px(34.0))
                        .h_full()
                        .flex()
                        .items_start()
                        .justify_center()
                        .pt(px(8.0))
                        .border_r_1()
                        .border_color(theme.tokens.border)
                        .bg(theme.tokens.card)
                        .child(
                            IconButton::new(IconSource::Named("panel-left".into()))
                                .size(px(26.0))
                                .variant(ButtonVariant::Ghost)
                                .on_click(move |_, _, cx| {
                                    let _ = app_weak_toggle_collapsed.update(cx, |state, cx| {
                                        state.toggle_sidebar(cx);
                                    });
                                }),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .child(editor_area),
            );

        let app_weak_prompt_cancel_backdrop = self.app_state.downgrade();
        let prompt_cancel_backdrop = move |_w: &mut Window, cx: &mut App| {
            let _ = app_weak_prompt_cancel_backdrop.update(cx, |state, cx| {
                state.pending_open_path = None;
                state.show_unsaved_prompt = false;
                state.pending_window_close = false;
                cx.notify();
            });
        };

        let app_weak_prompt_cancel_btn = self.app_state.downgrade();
        let app_weak_prompt_discard_btn = self.app_state.downgrade();
        let app_weak_prompt_save_btn = self.app_state.downgrade();

        let unsaved_prompt_modal = ModalDialog::new()
            .width(px(420.0))
            .on_backdrop_click(prompt_cancel_backdrop)
            .header(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p(px(16.0))
                    .border_b_1()
                    .border_color(theme.tokens.border)
                    .child(h4("Unsaved Changes")),
            )
            .content(
                div()
                    .p(px(16.0))
                    .child(body(if pending_window_close {
                        "You have unsaved changes. Save before closing?"
                    } else {
                        "You have unsaved changes. Save before opening?"
                    })),
            )
            .footer(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .p(px(16.0))
                    .border_t_1()
                    .border_color(theme.tokens.border)
                    .child(
                        Button::new("unsaved-cancel", "Cancel")
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _w, cx| {
                                let _ = app_weak_prompt_cancel_btn.update(cx, |state, cx| {
                                    state.pending_open_path = None;
                                    state.show_unsaved_prompt = false;
                                    state.pending_window_close = false;
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        Button::new("unsaved-discard", "Discard")
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _w, cx| {
                                let _ = app_weak_prompt_discard_btn.update(cx, |state, cx| {
                                    state.dirty = false;
                                    state.show_unsaved_prompt = false;
                                    if state.pending_window_close {
                                        state.pending_window_close = false;
                                        cx.quit();
                                    } else {
                                        if let Some(path) = state.pending_open_path.take() {
                                            match FileIo::open(&path) {
                                                Ok(doc) => state.load_document(doc, cx),
                                                Err(err) => eprintln!("Open error: {err}"),
                                            }
                                        }
                                        cx.notify();
                                    }
                                });
                            }),
                    )
                    .child(
                        Button::new("unsaved-save", "Save")
                            .variant(ButtonVariant::Default)
                            .on_click(move |_, _w, cx| {
                                let _ = app_weak_prompt_save_btn.update(cx, |state, cx| {
                                    state.save(cx);
                                    if !state.dirty {
                                        if state.pending_window_close {
                                            state.pending_window_close = false;
                                            state.show_unsaved_prompt = false;
                                            cx.quit();
                                        } else {
                                            if let Some(path) = state.pending_open_path.take() {
                                                match FileIo::open(&path) {
                                                    Ok(doc) => state.load_document(doc, cx),
                                                    Err(err) => eprintln!("Open error: {err}"),
                                                }
                                            }
                                            state.show_unsaved_prompt = false;
                                            cx.notify();
                                        }
                                    } else {
                                        cx.notify();
                                    }
                                });
                            }),
                    ),
            );

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.tokens.background)
            .child(self.toolbar.clone())
            .child(content_area)
            .child(status_bar)
            .when(auth_modal_open, |this| {
                this.child(self.github_auth_modal.clone())
            })
            .when(repo_add_modal_open, |this| {
                this.child(self.repo_add_modal.clone())
            })
            .when(show_unsaved_prompt, |this| {
                this.child(unsaved_prompt_modal)
            })
    }
}

