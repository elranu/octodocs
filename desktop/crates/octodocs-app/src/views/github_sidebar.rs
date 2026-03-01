//! GitHub Sidebar — repo selector shell and add-repo entrypoint.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use adabraka_ui::components::confirm_dialog::Dialog as ModalDialog;
use adabraka_ui::components::input::Input;
use adabraka_ui::components::input_state::InputState;
use adabraka_ui::overlays::context_menu::{
    ContextMenu as OverlayContextMenu, ContextMenuItem as OverlayContextMenuItem,
};
use adabraka_ui::prelude::*;
use octodocs_github::get_stored_token;

use crate::app_state::{AppState, PostAuthAction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CreateKind {
    File,
    Folder,
}

#[derive(Debug, Clone)]
struct CreateTarget {
    kind: CreateKind,
    parent: PathBuf,
}

#[derive(Debug, Clone)]
struct ExplorerEntry {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

pub struct GithubSidebar {
    app_state: Entity<AppState>,
    repo_dropdown_open: bool,
    expanded_dirs: HashSet<PathBuf>,
    entries_cache: HashMap<PathBuf, Vec<ExplorerEntry>>,
    refreshing_dirs: HashSet<PathBuf>,
    focused_dir: Option<PathBuf>,
    selected_file: Option<PathBuf>,
    file_context_menu: Option<(PathBuf, Point<Pixels>)>,
    rename_target: Option<PathBuf>,
    rename_input: Entity<InputState>,
    delete_target: Option<PathBuf>,
    create_target: Option<CreateTarget>,
    create_input: Entity<InputState>,
}

impl GithubSidebar {
    pub fn new(app_state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let create_input = cx.new(|cx| InputState::new(cx).placeholder("name"));
        let rename_input = cx.new(|cx| InputState::new(cx).placeholder("name"));
        Self {
            app_state,
            repo_dropdown_open: false,
            expanded_dirs: HashSet::new(),
            entries_cache: HashMap::new(),
            refreshing_dirs: HashSet::new(),
            focused_dir: None,
            selected_file: None,
            file_context_menu: None,
            rename_target: None,
            rename_input,
            delete_target: None,
            create_target: None,
            create_input,
        }
    }

    fn begin_rename_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("untitled")
            .to_string();
        self.rename_input = cx.new(|cx| {
            let mut input = InputState::new(cx).placeholder("name");
            input.content = stem.into();
            input
        });
        self.rename_target = Some(path);
        self.file_context_menu = None;
        cx.notify();
    }

    fn complete_rename_file(&mut self, value: SharedString, cx: &mut Context<Self>) {
        let Some(target) = self.rename_target.clone() else {
            return;
        };

        let raw = value.trim();
        if raw.is_empty() {
            return;
        }

        let stem = Path::new(raw)
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("untitled");

        let ext = target
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("md");
        let new_path = target.with_file_name(format!("{stem}.{ext}"));

        if new_path != target {
            if let Err(err) = std::fs::rename(&target, &new_path) {
                eprintln!("Rename error: {err}");
                return;
            }

            if self.selected_file.as_ref() == Some(&target) {
                self.selected_file = Some(new_path.clone());
            }

            self.app_state.update(cx, |state, cx| {
                if state.document.path.as_ref() == Some(&target) {
                    state.document.path = Some(new_path.clone());
                }
                state.sync_rename_to_github(target.clone(), new_path.clone(), cx);
                cx.notify();
            });

            if let Some(parent) = target.parent() {
                self.invalidate_entries_cache(Some(parent.to_path_buf()), cx);
            }
            if let Some(parent) = new_path.parent() {
                self.invalidate_entries_cache(Some(parent.to_path_buf()), cx);
            }
        }

        self.rename_target = None;
        cx.notify();
    }

    fn duplicate_file(&mut self, source: PathBuf, cx: &mut Context<Self>) {
        let stem = source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("copy")
            .to_string();
        let ext = source
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("md")
            .to_string();
        let parent = source
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        let mut candidate = parent.join(format!("{stem} copy.{ext}"));
        let mut idx = 2usize;
        while candidate.exists() {
            candidate = parent.join(format!("{stem} copy {idx}.{ext}"));
            idx += 1;
        }

        if let Err(err) = std::fs::copy(&source, &candidate) {
            eprintln!("Duplicate error: {err}");
            return;
        }

        if let Some(parent) = source.parent() {
            self.invalidate_entries_cache(Some(parent.to_path_buf()), cx);
        }

        self.selected_file = Some(candidate.clone());
        self.begin_rename_file(candidate, cx);
    }

    fn confirm_delete_file(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.delete_target.clone() else {
            return;
        };

        if let Err(err) = std::fs::remove_file(&target) {
            eprintln!("Delete error: {err}");
            return;
        }

        if let Some(parent) = target.parent() {
            self.invalidate_entries_cache(Some(parent.to_path_buf()), cx);
        }

        if self.selected_file.as_ref() == Some(&target) {
            self.selected_file = None;
        }
        if self.rename_target.as_ref() == Some(&target) {
            self.rename_target = None;
        }

        self.app_state.update(cx, |state, cx| {
            if state.document.path.as_ref() == Some(&target) {
                state.document.path = None;
                state.dirty = true;
            }
            cx.notify();
        });

        self.delete_target = None;
        self.file_context_menu = None;
        cx.notify();
    }

    fn active_index(&self, bindings_len: usize, active_binding_idx: Option<usize>) -> Option<usize> {
        active_binding_idx
            .filter(|idx| *idx < bindings_len)
            .or_else(|| (bindings_len > 0).then_some(0))
    }

    fn begin_create(&mut self, kind: CreateKind, parent: PathBuf, cx: &mut Context<Self>) {
        let placeholder = match kind {
            CreateKind::File => "new-file.md",
            CreateKind::Folder => "new-folder",
        };
        self.create_input = cx.new(|cx| InputState::new(cx).placeholder(placeholder));
        self.create_target = Some(CreateTarget { kind, parent });
        cx.notify();
    }

    fn complete_create(&mut self, value: SharedString, cx: &mut Context<Self>) {
        let Some(target) = self.create_target.clone() else {
            return;
        };

        let raw_name = value.trim();
        if raw_name.is_empty() {
            return;
        }

        let name = raw_name.to_string();
        let full_path = match target.kind {
            CreateKind::File => {
                let stem = std::path::Path::new(&name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("untitled");
                target.parent.join(format!("{stem}.md"))
            }
            CreateKind::Folder => target.parent.join(name),
        };

        let result = match target.kind {
            CreateKind::File => std::fs::write(&full_path, ""),
            CreateKind::Folder => std::fs::create_dir(&full_path),
        };

        if let Err(err) = result {
            eprintln!("Create error: {err}");
            return;
        }

        let parent_dir = target.parent.clone();
        self.expanded_dirs.insert(parent_dir.clone());
        self.invalidate_entries_cache(Some(parent_dir), cx);
        if matches!(target.kind, CreateKind::Folder) {
            self.invalidate_entries_cache(Some(full_path), cx);
        }
        self.create_target = None;
        cx.notify();
    }

    fn scan_entries(dir: &Path) -> Vec<ExplorerEntry> {
        let Ok(read_dir) = std::fs::read_dir(dir) else {
            return vec![];
        };

        let mut entries = read_dir
            .filter_map(|item| {
                let entry = item.ok()?;
                let path = entry.path();
                let file_type = entry.file_type().ok()?;
                let is_dir = file_type.is_dir();

                if !is_dir {
                    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
                    const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];
                    if extension != "md" && !IMAGE_EXTS.contains(&extension.as_str()) {
                        return None;
                    }
                }

                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.to_string())?;

                Some(ExplorerEntry { path, name, is_dir })
            })
            .collect::<Vec<_>>();

        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
        });

        entries
    }

    fn list_entries(&self, dir: &Path) -> Vec<ExplorerEntry> {
        self.entries_cache.get(dir).cloned().unwrap_or_default()
    }

    fn refresh_entries_for_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        if self.refreshing_dirs.contains(&dir) {
            return;
        }
        self.refreshing_dirs.insert(dir.clone());

        cx.spawn(async move |this, cx| {
            let refresh_dir = dir.clone();
            let entries = cx
                .background_executor()
                .spawn(async move { Self::scan_entries(&refresh_dir) })
                .await;

            let _ = this.update(cx, |sidebar, cx| {
                sidebar.entries_cache.insert(dir.clone(), entries);
                sidebar.refreshing_dirs.remove(&dir);
                cx.notify();
            });
        })
        .detach();
    }

    fn invalidate_entries_cache(&mut self, dir: Option<PathBuf>, cx: &mut Context<Self>) {
        match dir {
            Some(dir) => {
                self.entries_cache.remove(&dir);
                self.refresh_entries_for_dir(dir, cx);
            }
            None => {
                self.entries_cache.clear();
                self.refreshing_dirs.clear();
                if let Some(root) = self.current_local_root(cx) {
                    self.refresh_entries_for_dir(root.clone(), cx);
                    let expanded: Vec<PathBuf> = self
                        .expanded_dirs
                        .iter()
                        .filter(|p| p.starts_with(&root))
                        .cloned()
                        .collect();
                    for dir in expanded {
                        self.refresh_entries_for_dir(dir, cx);
                    }
                }
            }
        }
    }

    fn current_local_root(&self, cx: &mut Context<Self>) -> Option<PathBuf> {
        let app = self.app_state.read(cx);
        let idx = self.active_index(app.github_bindings.len(), app.active_binding_idx)?;
        app.github_bindings.get(idx).map(|b| b.local_root.clone())
    }

    fn ensure_visible_dirs_cached(&mut self, local_root: &Path, cx: &mut Context<Self>) {
        if !self.entries_cache.contains_key(local_root)
            && !self.refreshing_dirs.contains(local_root)
        {
            self.refresh_entries_for_dir(local_root.to_path_buf(), cx);
        }

        let expanded: Vec<PathBuf> = self
            .expanded_dirs
            .iter()
            .filter(|p| p.starts_with(local_root))
            .cloned()
            .collect();

        for dir in expanded {
            if !self.entries_cache.contains_key(&dir) && !self.refreshing_dirs.contains(&dir) {
                self.refresh_entries_for_dir(dir, cx);
            }
        }
    }

    fn push_tree_rows(
        &self,
        dir: &Path,
        depth: usize,
        rows: &mut Vec<AnyElement>,
        weak: &gpui::WeakEntity<Self>,
        current_doc_path: Option<&Path>,
    ) {
        let theme = use_theme();
        for entry in self.list_entries(dir) {
            if entry.is_dir {
                let path = entry.path.clone();
                let name = entry.name.clone();
                let expanded = self.expanded_dirs.contains(&path);
                let row_weak = weak.clone();
                let indent = depth as f32 * 12.0;

                rows.push(
                    div()
                        .pl(px(indent))
                        .pr(px(6.0))
                        .py(px(4.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.tokens.accent))
                        .rounded(px(6.0))
                        .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                            let _ = row_weak.update(cx, |sidebar, cx| {
                                sidebar.focused_dir = Some(path.clone());
                                if sidebar.expanded_dirs.contains(&path) {
                                    sidebar.expanded_dirs.remove(&path);
                                } else {
                                    sidebar.expanded_dirs.insert(path.clone());
                                    sidebar.refresh_entries_for_dir(path.clone(), cx);
                                }
                                cx.notify();
                            });
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    Icon::new(IconSource::Named(
                                        if expanded { "chevron-down" } else { "chevron-right" }
                                            .into(),
                                    ))
                                    .size_3(),
                                )
                                .child(Icon::new(IconSource::Named("folder".into())).size_3())
                                .child(body_small(name)),
                        )
                        .into_any_element(),
                );

                if expanded {
                    self.push_tree_rows(&entry.path, depth + 1, rows, weak, current_doc_path);
                }
            } else {
                let path = entry.path.clone();
                let name = entry.name.clone();
                let row_weak = weak.clone();
                let right_click_weak = weak.clone();
                let path_for_left = path.clone();
                let path_for_right = path.clone();
                let indent = depth as f32 * 12.0;
                let is_renaming = self.rename_target.as_ref() == Some(&path);
                // Highlight when this row is the sidebar selection OR the currently open document.
                let pending_selected = self.selected_file.as_deref();
                let has_pending_switch = pending_selected.is_some() && pending_selected != current_doc_path;
                let is_active = current_doc_path == Some(path.as_path()) && !has_pending_switch;
                let is_selected = pending_selected == Some(path.as_path()) || is_active;
                let is_pending_loading = has_pending_switch && pending_selected == Some(path.as_path());

                if is_renaming {
                    let rename_weak = weak.clone();
                    let cancel_weak = weak.clone();
                    rows.push(
                        div()
                            .pl(px(indent + 12.0))
                            .pr(px(6.0))
                            .py(px(4.0))
                            .child(
                                Input::new(&self.rename_input)
                                    .placeholder("new name")
                                    .on_enter(move |value, cx| {
                                        let _ = rename_weak.update(cx, |sidebar, cx| {
                                            sidebar.complete_rename_file(value, cx);
                                        });
                                    }),
                            )
                            .child(
                                Button::new("cancel-rename-file", "Cancel")
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .on_click(move |_, _, cx| {
                                        let _ = cancel_weak.update(cx, |sidebar, cx| {
                                            sidebar.rename_target = None;
                                            cx.notify();
                                        });
                                    }),
                            )
                            .into_any_element(),
                    );
                } else {
                    rows.push(
                        div()
                            .pl(px(indent + 12.0))
                            .pr(px(6.0))
                            .py(px(4.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.tokens.accent))
                            .when(is_selected && !is_active, |s| {
                                s.bg(theme.tokens.accent)
                                    .border_l_2()
                                    .border_color(theme.tokens.primary.opacity(0.6))
                            })
                            .when(is_active, |s| {
                                s.bg(theme.tokens.accent)
                                    .border_l_2()
                                    .border_color(theme.tokens.primary)
                            })
                            .rounded(px(6.0))
                            .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                                let _ = row_weak.update(cx, |sidebar, cx| {
                                    sidebar.selected_file = Some(path_for_left.clone());
                                    cx.notify(); // highlight the row immediately, before the file finishes loading
                                    let ext = path_for_left
                                        .extension()
                                        .and_then(|e| e.to_str())
                                        .map(|e| e.to_ascii_lowercase())
                                        .unwrap_or_default();
                                    if ext == "md" {
                                        // Defer open to the next tick so the selection highlight
                                        // is painted before any state transitions for loading begin.
                                        let app_state = sidebar.app_state.clone();
                                        let path = path_for_left.clone();
                                        cx.spawn(async move |_, cx| {
                                            let _ = cx
                                                .background_executor()
                                                .spawn(async move {
                                                    std::thread::sleep(Duration::from_millis(16));
                                                })
                                                .await;
                                            let _ = app_state.update(cx, |state, cx| {
                                                state.open_file_from_sidebar(path, cx);
                                            });
                                        })
                                        .detach();
                                    }
                                });
                            })
                            .on_mouse_down(gpui::MouseButton::Right, move |event, _, cx| {
                                let click_position = event.position;
                                let _ = right_click_weak.update(cx, |sidebar, cx| {
                                    sidebar.selected_file = Some(path_for_right.clone());
                                    sidebar.file_context_menu = Some((path_for_right.clone(), click_position));
                                    cx.notify();
                                });
                            })
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(6.0))
                                            .child(Icon::new(IconSource::Named("file".into())).size_3())
                                            .child(body_small(name)),
                                    )
                                    .when(is_pending_loading, |s| {
                                        s.child(
                                            Icon::new(IconSource::Named("loader".into()))
                                                .size_3()
                                                .color(theme.tokens.muted_foreground),
                                        )
                                    }),
                            )
                            .into_any_element(),
                    );
                }
            }
        }
    }

    fn open_add_repo_flow(&mut self, cx: &mut Context<Self>) {
        let token = get_stored_token().ok().flatten();
        self.app_state.update(cx, |state, cx| {
            if token.is_some() {
                state.pending_post_auth_action = None;
                state.repo_add_modal_open = true;
            } else {
                state.pending_post_auth_action = Some(PostAuthAction::AddRepo);
                state.auth_modal_open = true;
            }
            cx.notify();
        });
    }

    fn set_active_binding(&mut self, idx: usize, cx: &mut Context<Self>) {
        self.app_state.update(cx, |state, cx| {
            state.set_active_binding_idx(idx, cx);
        });
        let selected_root = self
            .app_state
            .read(cx)
            .github_bindings
            .get(idx)
            .map(|binding| binding.local_root.clone());
        if let Some(root) = selected_root {
            self.focused_dir = Some(root);
            self.expanded_dirs.clear();
        }
        self.invalidate_entries_cache(None, cx);
        self.repo_dropdown_open = false;
        cx.notify();
    }
}

impl Render for GithubSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = use_theme();
        let weak = cx.entity().downgrade();

        let app = self.app_state.read(cx);
        let bindings = app.github_bindings.clone();
        let active_idx = self.active_index(bindings.len(), app.active_binding_idx);
        let _ = app;

        let repo_selector: AnyElement = if bindings.is_empty() {
            let add_weak = weak.clone();
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .p(px(12.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(theme.tokens.border)
                .child(body("No repos connected").color(theme.tokens.muted_foreground))
                .child(
                    Button::new("sidebar-add-first-repo", "+")
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _w, cx| {
                            let _ = add_weak.update(cx, |sidebar, cx| sidebar.open_add_repo_flow(cx));
                        }),
                )
                .into_any_element()
        } else {
            let active = active_idx.unwrap_or(0);
            let active_binding = &bindings[active];
            let active_label = format!(
                "{}/{}:{}",
                active_binding.config.owner, active_binding.config.repo, active_binding.config.branch
            );

            let toggle_weak = weak.clone();

            let selector = div()
                .flex()
                .items_center()
                .child(
                    div()
                        .flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .px(px(10.0))
                        .py(px(8.0))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(theme.tokens.border)
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.tokens.accent))
                        .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                            let _ = toggle_weak.update(cx, |sidebar, cx| {
                                sidebar.repo_dropdown_open = !sidebar.repo_dropdown_open;
                                cx.notify();
                            });
                        })
                        .child(body_small(active_label))
                        .child(Icon::new(IconSource::Named("chevron-down".into())).size_3()),
                );

            let dropdown = if self.repo_dropdown_open {
                let weak_for_rows = weak.clone();
                div()
                    .mt(px(6.0))
                    .border_1()
                    .border_color(theme.tokens.border)
                    .rounded(px(8.0))
                    .max_h(px(240.0))
                    .children(bindings.iter().enumerate().map(|(idx, binding)| {
                        let row_weak = weak_for_rows.clone();
                        let label = format!(
                            "{}/{}:{}",
                            binding.config.owner, binding.config.repo, binding.config.branch
                        );
                        div()
                            .px(px(10.0))
                            .py(px(8.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.tokens.accent))
                            .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                                let _ = row_weak.update(cx, |sidebar, cx| {
                                    sidebar.set_active_binding(idx, cx)
                                });
                            })
                            .child(body_small(label))
                    }))
                    .into_any_element()
            } else {
                div().into_any_element()
            };

            div().child(selector).child(dropdown).into_any_element()
        };

        let add_weak = weak.clone();
        let toggle_weak = self.app_state.downgrade();
        let context_menu_overlay: AnyElement = if let Some((target_path, click_position)) = self.file_context_menu.clone() {
            let rename_weak = weak.clone();
            let duplicate_weak = weak.clone();
            let delete_weak = weak.clone();
            let close_weak = weak.clone();
            let target_for_rename = target_path.clone();
            let target_for_duplicate = target_path.clone();
            let target_for_delete = target_path.clone();

            OverlayContextMenu::new(click_position)
                .items(vec![
                    OverlayContextMenuItem::new("rename-file", "Rename")
                        .on_click(move |_, cx| {
                            let _ = rename_weak.update(cx, |sidebar, cx| {
                                sidebar.begin_rename_file(target_for_rename.clone(), cx);
                            });
                        }),
                    OverlayContextMenuItem::new("duplicate-file", "Duplicate")
                        .on_click(move |_, cx| {
                            let _ = duplicate_weak.update(cx, |sidebar, cx| {
                                sidebar.duplicate_file(target_for_duplicate.clone(), cx);
                            });
                        }),
                    OverlayContextMenuItem::separator(),
                    OverlayContextMenuItem::new("delete-file", "Delete")
                        .on_click(move |_, cx| {
                            let _ = delete_weak.update(cx, |sidebar, cx| {
                                sidebar.delete_target = Some(target_for_delete.clone());
                                sidebar.file_context_menu = None;
                                cx.notify();
                            });
                        }),
                ])
                .on_close(move |_, cx| {
                    let _ = close_weak.update(cx, |sidebar, cx| {
                        sidebar.file_context_menu = None;
                        cx.notify();
                    });
                })
                .into_any_element()
        } else {
            div().into_any_element()
        };

        let delete_confirm_overlay: AnyElement = if self.delete_target.is_some() {
            let confirm_weak = weak.clone();
            let cancel_weak = weak.clone();
            let cancel_weak_backdrop = cancel_weak.clone();

            ModalDialog::new()
                .width(px(380.0))
                .on_backdrop_click(move |_, cx| {
                    let _ = cancel_weak_backdrop.update(cx, |sidebar, cx| {
                        sidebar.delete_target = None;
                        cx.notify();
                    });
                })
                .header(
                    div()
                        .p(px(14.0))
                        .border_b_1()
                        .border_color(theme.tokens.border)
                        .child(h5("Delete File")),
                )
                .content(
                    div()
                        .p(px(14.0))
                        .child(body("Are you sure you want to delete this file?")),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.0))
                        .p(px(12.0))
                        .border_t_1()
                        .border_color(theme.tokens.border)
                        .child(
                            Button::new("cancel-delete-file", "Cancel")
                                .variant(ButtonVariant::Ghost)
                                .on_click(move |_, _, cx| {
                                    let _ = cancel_weak.update(cx, |sidebar, cx| {
                                        sidebar.delete_target = None;
                                        cx.notify();
                                    });
                                }),
                        )
                        .child(
                            Button::new("confirm-delete-file", "Delete")
                                .variant(ButtonVariant::Destructive)
                                .on_click(move |_, _, cx| {
                                    let _ = confirm_weak.update(cx, |sidebar, cx| {
                                        sidebar.confirm_delete_file(cx);
                                    });
                                }),
                        ),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        };

        let file_explorer: AnyElement = if let Some(active) = active_idx {
            let local_root = bindings[active].local_root.clone();
            if !local_root.exists() {
                let _ = std::fs::create_dir_all(&local_root);
            }
            self.ensure_visible_dirs_cached(&local_root, cx);
            let mut rows = Vec::new();
            // Pass the currently-open document path so every render cycle the active
            // file is highlighted regardless of how it was opened.
            let current_doc_path = self.app_state.read(cx).document.path.clone();
            self.push_tree_rows(&local_root, 0, &mut rows, &weak, current_doc_path.as_deref());

            let current_folder = self
                .focused_dir
                .clone()
                .filter(|path| path.starts_with(&local_root))
                .unwrap_or(local_root.clone());
            let current_folder_name = current_folder
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.to_string())
                .unwrap_or_else(|| current_folder.display().to_string());

            let new_file_weak = weak.clone();
            let new_folder_weak = weak.clone();
            let cancel_create_weak = weak.clone();
            let submit_create_weak = weak.clone();
            let local_root_for_file = local_root.clone();
            let local_root_for_folder = local_root.clone();

            let creation_input = if self.create_target.is_some() {
                let label = match self.create_target.as_ref().map(|t| t.kind) {
                    Some(CreateKind::File) => "New file name",
                    Some(CreateKind::Folder) => "New folder name",
                    None => "Name",
                };

                div()
                    .mt(px(8.0))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(body_small(label).color(theme.tokens.muted_foreground))
                    .child(
                        Input::new(&self.create_input)
                            .placeholder("name")
                            .on_enter(move |value, cx| {
                                let _ = submit_create_weak
                                    .update(cx, |sidebar, cx| sidebar.complete_create(value, cx));
                            }),
                    )
                    .child(
                        Button::new("cancel-create", "Cancel")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .on_click(move |_, _w, cx| {
                                let _ = cancel_create_weak.update(cx, |sidebar, cx| {
                                    sidebar.create_target = None;
                                    cx.notify();
                                });
                            }),
                    )
                    .into_any_element()
            } else {
                div().into_any_element()
            };

            div()
                .flex_1()
                .rounded(px(8.0))
                .border_1()
                .border_color(theme.tokens.border)
                .p(px(10.0))
                .child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(
                            div()
                                .id("new-md-file")
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .px(px(10.0))
                                .py(px(6.0))
                                .rounded(px(6.0))
                                .cursor_pointer()
                                .hover(|s| s.bg(theme.tokens.accent))
                                .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                                    let _ = new_file_weak.update(cx, |sidebar, cx| {
                                        sidebar.begin_create(
                                            CreateKind::File,
                                            sidebar
                                                .focused_dir
                                                .clone()
                                                .unwrap_or_else(|| local_root_for_file.clone()),
                                            cx,
                                        );
                                    });
                                })
                                .child(Icon::new(IconSource::Named("file".into())).size_3())
                                .child(body_small("New File")),
                        )
                        .child(
                            div()
                                .id("new-folder")
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .px(px(10.0))
                                .py(px(6.0))
                                .rounded(px(6.0))
                                .cursor_pointer()
                                .hover(|s| s.bg(theme.tokens.accent))
                                .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                                    let _ = new_folder_weak.update(cx, |sidebar, cx| {
                                        sidebar.begin_create(
                                            CreateKind::Folder,
                                            sidebar
                                                .focused_dir
                                                .clone()
                                                .unwrap_or_else(|| local_root_for_folder.clone()),
                                            cx,
                                        );
                                    });
                                })
                                .child(Icon::new(IconSource::Named("folder".into())).size_3())
                                .child(body_small("New Folder")),
                        ),
                )
                .child(
                    div()
                        .mt(px(8.0))
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(body_small("Files"))
                        .child(
                            body_small(current_folder_name)
                                .color(theme.tokens.muted_foreground),
                        ),
                )
                .child(div().mt(px(8.0)).flex().flex_col().gap(px(2.0)).children(rows))
                .child(creation_input)
                .into_any_element()
        } else {
            div()
                .flex_1()
                .rounded(px(8.0))
                .border_1()
                .border_color(theme.tokens.border)
                .p(px(10.0))
                .child(
                    body_small("Connect a repository to browse local files.")
                        .color(theme.tokens.muted_foreground),
                )
                .into_any_element()
        };

        div()
            .id("github-sidebar")
            .w(px(260.0))
            .h_full()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .p(px(12.0))
            .border_l_1()
            .border_color(theme.tokens.border)
            .bg(theme.tokens.card)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(h5("Repositories"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(
                                IconButton::new(IconSource::Named("plus".into()))
                                    .size(px(26.0))
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(move |_, _w, cx| {
                                        let _ = add_weak
                                            .update(cx, |sidebar, cx| sidebar.open_add_repo_flow(cx));
                                    }),
                            )
                            .child(
                                IconButton::new(IconSource::Named("panel-left".into()))
                                    .size(px(26.0))
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(move |_, _, cx| {
                                        let _ = toggle_weak.update(cx, |state, cx| {
                                            state.toggle_sidebar(cx);
                                        });
                                    }),
                            ),
                    ),
            )
            .child(repo_selector)
            .child(file_explorer)
            .child(context_menu_overlay)
            .child(delete_confirm_overlay)
    }
}
