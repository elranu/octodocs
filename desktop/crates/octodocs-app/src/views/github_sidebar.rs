//! GitHub Sidebar — repo selector shell and add-repo entrypoint.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use adabraka_ui::components::input::Input;
use adabraka_ui::components::input_state::InputState;
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
    focused_dir: Option<PathBuf>,
    create_target: Option<CreateTarget>,
    create_input: Entity<InputState>,
}

impl GithubSidebar {
    pub fn new(app_state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let create_input = cx.new(|cx| InputState::new(cx).placeholder("name"));
        Self {
            app_state,
            repo_dropdown_open: false,
            expanded_dirs: HashSet::new(),
            focused_dir: None,
            create_target: None,
            create_input,
        }
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

        let mut name = raw_name.to_string();
        let full_path = match target.kind {
            CreateKind::File => {
                if !name.to_ascii_lowercase().ends_with(".md") {
                    name.push_str(".md");
                }
                target.parent.join(name)
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

        self.expanded_dirs.insert(target.parent);
        self.create_target = None;
        cx.notify();
    }

    fn list_entries(&self, dir: &Path) -> Vec<ExplorerEntry> {
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
                    if extension != "md" {
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

    fn push_tree_rows(
        &self,
        dir: &Path,
        depth: usize,
        rows: &mut Vec<AnyElement>,
        weak: &gpui::WeakEntity<Self>,
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
                    self.push_tree_rows(&entry.path, depth + 1, rows, weak);
                }
            } else {
                let path = entry.path.clone();
                let name = entry.name.clone();
                let row_weak = weak.clone();
                let indent = depth as f32 * 12.0;

                rows.push(
                    div()
                        .pl(px(indent + 12.0))
                        .pr(px(6.0))
                        .py(px(4.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.tokens.accent))
                        .rounded(px(6.0))
                        .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                            let _ = row_weak.update(cx, |sidebar, cx| {
                                sidebar.app_state.update(cx, |state, cx| {
                                    state.open_file_from_sidebar(path.clone(), cx);
                                });
                            });
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                            .child(Icon::new(IconSource::Named("file".into())).size_3())
                                .child(body_small(name)),
                        )
                        .into_any_element(),
                );
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
            state.active_binding_idx = Some(idx);
            cx.notify();
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
        drop(app);

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
            let add_weak = weak.clone();

            let selector = div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .flex_1()
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
                )
                .child(
                        IconButton::new(IconSource::Named("plus".into()))
                        .size(px(28.0))
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _w, cx| {
                            let _ = add_weak.update(cx, |sidebar, cx| sidebar.open_add_repo_flow(cx));
                        }),
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

        let file_explorer: AnyElement = if let Some(active) = active_idx {
            let local_root = bindings[active].local_root.clone();
            if !local_root.exists() {
                let _ = std::fs::create_dir_all(&local_root);
            }
            let mut rows = Vec::new();
            self.push_tree_rows(&local_root, 0, &mut rows, &weak);

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
                        .items_center()
                        .justify_between()
                        .child(body_small("Files"))
                        .child(
                            body_small(current_folder_name)
                                .color(theme.tokens.muted_foreground),
                        ),
                )
                .child(div().mt(px(8.0)).flex().flex_col().gap(px(2.0)).children(rows))
                .child(
                    div()
                        .mt(px(10.0))
                        .pt(px(8.0))
                        .border_t_1()
                        .border_color(theme.tokens.border)
                        .flex()
                        .gap(px(8.0))
                        .child(
                            Button::new("new-md-file", "+ New File")
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .on_click(move |_, _w, cx| {
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
                                }),
                        )
                        .child(
                            Button::new("new-folder", "+ Folder")
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .on_click(move |_, _w, cx| {
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
                                }),
                        ),
                )
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
                        IconButton::new(IconSource::Named("plus".into()))
                            .size(px(26.0))
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _w, cx| {
                                let _ = add_weak
                                    .update(cx, |sidebar, cx| sidebar.open_add_repo_flow(cx));
                            }),
                    ),
            )
            .child(repo_selector)
            .child(file_explorer)
    }
}
