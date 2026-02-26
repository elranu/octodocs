//! Repo Add Modal — repository/branch/folder wizard for creating a sync binding.

use std::path::PathBuf;

use adabraka_ui::components::confirm_dialog::Dialog as ModalDialog;
use adabraka_ui::components::input::Input;
use adabraka_ui::components::input_state::InputState;
use adabraka_ui::prelude::*;
use gpui::Task;
use octodocs_github::{
    list_branches, list_folder, list_repos, pull_markdown_files, BranchInfo, FolderEntry,
    GitHubSyncConfig, RepoInfo,
};

use crate::app_state::AppState;

fn default_local_root_for_repo(repo_name: &str) -> PathBuf {
    let base = dirs::document_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Documents")))
        .unwrap_or_else(|| PathBuf::from("."));

    base.join(repo_name)
}

#[derive(Debug, Clone)]
pub enum WizardState {
    RepoSelect {
        repos: Vec<RepoInfo>,
        loading: bool,
    },
    BranchSelect {
        repo: RepoInfo,
        branches: Vec<BranchInfo>,
        loading: bool,
    },
    FolderSelect {
        repo: RepoInfo,
        branch: String,
        folders: Vec<FolderEntry>,
        current_path: String,
        loading: bool,
    },
    Confirm {
        config: GitHubSyncConfig,
    },
    Error {
        message: String,
    },
    Applying,
}

pub struct RepoAddModal {
    app_state: Entity<AppState>,
    auth_token: String,
    initialized: bool,
    pub repo_search_input: Entity<InputState>,
    pub selected_local_root: Option<PathBuf>,
    pub state: WizardState,
    _task: Option<Task<()>>,
}

impl RepoAddModal {
    fn remote_to_local_path(local_root: &std::path::Path, base_folder: &str, remote_path: &str) -> PathBuf {
        let normalized_remote = remote_path.trim_start_matches('/');
        let base = base_folder.trim_matches('/');

        let rel = if base.is_empty() {
            normalized_remote.to_string()
        } else {
            let prefix = format!("{base}/");
            normalized_remote
                .strip_prefix(&prefix)
                .unwrap_or(normalized_remote)
                .to_string()
        };

        local_root.join(rel)
    }

    fn confirm_default_for_empty_repo(&mut self, repo: RepoInfo, cx: &mut Context<Self>) {
        self.selected_local_root = self
            .app_state
            .read(cx)
            .document
            .path
            .as_ref()
            .and_then(|path| path.parent().map(|parent| parent.to_path_buf()));

        self.state = WizardState::Confirm {
            config: GitHubSyncConfig {
                owner: repo.owner,
                repo: repo.name,
                branch: "main".to_string(),
                folder: String::new(),
            },
        };
        cx.notify();
    }

    pub fn new(app_state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let repo_search_input = cx.new(|cx| InputState::new(cx).placeholder("Search repositories..."));

        Self {
            app_state,
            auth_token: String::new(),
            initialized: false,
            repo_search_input,
            selected_local_root: None,
            state: WizardState::RepoSelect {
                repos: vec![],
                loading: true,
            },
            _task: None,
        }
    }

    pub fn set_auth_token(&mut self, auth_token: String, cx: &mut Context<Self>) {
        if self.auth_token == auth_token {
            return;
        }
        self.auth_token = auth_token;
        self.initialized = false;
        self.state = WizardState::RepoSelect {
            repos: vec![],
            loading: true,
        };
        cx.notify();
    }

    pub fn init(&mut self, cx: &mut Context<Self>) {
        self.load_repos(cx);
    }

    fn load_repos(&mut self, cx: &mut Context<Self>) {
        let token = self.auth_token.clone();
        self.state = WizardState::RepoSelect {
            repos: vec![],
            loading: true,
        };
        cx.notify();

        let weak = cx.entity().downgrade();
        self._task = Some(cx.spawn(async move |_, cx| {
            let repos = cx
                .background_executor()
                .spawn(async move { list_repos(&token) })
                .await;

            let _ = weak.update(cx, |modal, cx| match repos {
                Ok(repos) => {
                    modal.state = WizardState::RepoSelect {
                        repos,
                        loading: false,
                    };
                    cx.notify();
                }
                Err(e) => {
                    modal.state = WizardState::Error {
                        message: format!("Failed to load repos: {e}"),
                    };
                    cx.notify();
                }
            });
        }));
    }

    pub fn select_repo(&mut self, repo: RepoInfo, cx: &mut Context<Self>) {
        let token = self.auth_token.clone();

        self.state = WizardState::BranchSelect {
            repo: repo.clone(),
            branches: vec![],
            loading: true,
        };
        cx.notify();

        let weak = cx.entity().downgrade();
        let owner = repo.owner.clone();
        let name = repo.name.clone();

        self._task = Some(cx.spawn(async move |_, cx| {
            let branches = cx
                .background_executor()
                .spawn(async move { list_branches(&token, &owner, &name) })
                .await;

            let _ = weak.update(cx, |modal, cx| match branches {
                Ok(branches) => {
                    if branches.is_empty() {
                        modal.confirm_default_for_empty_repo(repo, cx);
                        return;
                    }

                    modal.state = WizardState::BranchSelect {
                        repo,
                        branches,
                        loading: false,
                    };
                    cx.notify();
                }
                Err(e) => {
                    let message = e.to_string().to_ascii_lowercase();
                    if message.contains("409")
                        || message.contains("empty")
                        || message.contains("no commits")
                    {
                        modal.confirm_default_for_empty_repo(repo, cx);
                    } else {
                        modal.state = WizardState::Error {
                            message: format!("Failed to load branches: {e}"),
                        };
                        cx.notify();
                    }
                }
            });
        }));
    }

    pub fn select_branch(&mut self, repo: RepoInfo, branch: String, cx: &mut Context<Self>) {
        let token = self.auth_token.clone();

        self.state = WizardState::FolderSelect {
            repo: repo.clone(),
            branch: branch.clone(),
            folders: vec![],
            current_path: String::new(),
            loading: true,
        };
        cx.notify();

        let weak = cx.entity().downgrade();
        let owner = repo.owner.clone();
        let name = repo.name.clone();
        let branch_clone = branch.clone();

        self._task = Some(cx.spawn(async move |_, cx| {
            let folders = cx
                .background_executor()
                .spawn(async move { list_folder(&token, &owner, &name, &branch_clone, "") })
                .await;

            let _ = weak.update(cx, |modal, cx| match folders {
                Ok(folders) => {
                    modal.state = WizardState::FolderSelect {
                        repo,
                        branch,
                        folders,
                        current_path: String::new(),
                        loading: false,
                    };
                    cx.notify();
                }
                Err(e) => {
                    modal.state = WizardState::Error {
                        message: format!("Failed to load folders: {e}"),
                    };
                    cx.notify();
                }
            });
        }));
    }

    pub fn enter_folder(&mut self, folder_path: String, cx: &mut Context<Self>) {
        let token = self.auth_token.clone();
        let (repo, branch) = match &self.state {
            WizardState::FolderSelect { repo, branch, .. } => (repo.clone(), branch.clone()),
            _ => return,
        };

        self.state = WizardState::FolderSelect {
            repo: repo.clone(),
            branch: branch.clone(),
            folders: vec![],
            current_path: folder_path.clone(),
            loading: true,
        };
        cx.notify();

        let weak = cx.entity().downgrade();
        let owner = repo.owner.clone();
        let name = repo.name.clone();
        let branch_for_request = branch.clone();
        let folder_path_for_request = folder_path.clone();

        self._task = Some(cx.spawn(async move |_, cx| {
            let folders = cx
                .background_executor()
                .spawn(async move {
                    list_folder(&token, &owner, &name, &branch_for_request, &folder_path_for_request)
                })
                .await;

            let _ = weak.update(cx, |modal, cx| match folders {
                Ok(folders) => {
                    modal.state = WizardState::FolderSelect {
                        repo,
                        branch,
                        folders,
                        current_path: folder_path,
                        loading: false,
                    };
                    cx.notify();
                }
                Err(e) => {
                    modal.state = WizardState::Error {
                        message: format!("Failed to load folders: {e}"),
                    };
                    cx.notify();
                }
            });
        }));
    }

    pub fn confirm_folder(&mut self, cx: &mut Context<Self>) {
        let config = match &self.state {
            WizardState::FolderSelect {
                repo,
                branch,
                current_path,
                ..
            } => GitHubSyncConfig {
                owner: repo.owner.clone(),
                repo: repo.name.clone(),
                branch: branch.clone(),
                folder: current_path.clone(),
            },
            _ => return,
        };

        self.selected_local_root = self
            .app_state
            .read(cx)
            .document
            .path
            .as_ref()
            .and_then(|path| path.parent().map(|parent| parent.to_path_buf()));

        self.state = WizardState::Confirm { config };
        cx.notify();
    }

    pub fn choose_local_root(&mut self, repo_name: String, cx: &mut Context<Self>) {
        let default_dir = default_local_root_for_repo(&repo_name);
        if let Err(err) = std::fs::create_dir_all(&default_dir) {
            self.state = WizardState::Error {
                message: format!("Failed to create default local folder: {err}"),
            };
            cx.notify();
            return;
        }

        if let Some(path) = rfd::FileDialog::new().set_directory(&default_dir).pick_folder() {
            self.selected_local_root = Some(path);
            cx.notify();
        }
    }

    pub fn use_default_local_root(&mut self, repo_name: String, cx: &mut Context<Self>) {
        let default_dir = default_local_root_for_repo(&repo_name);
        match std::fs::create_dir_all(&default_dir) {
            Ok(_) => {
                self.selected_local_root = Some(default_dir);
                cx.notify();
            }
            Err(err) => {
                self.state = WizardState::Error {
                    message: format!("Failed to create local folder: {err}"),
                };
                cx.notify();
            }
        }
    }

    pub fn apply_config(&mut self, cx: &mut Context<Self>) {
        let config = match &self.state {
            WizardState::Confirm { config } => config.clone(),
            _ => return,
        };

        let Some(local_root) = self.selected_local_root.clone() else {
            self.state = WizardState::Error {
                message: "Select a local folder for this repository mapping".to_string(),
            };
            cx.notify();
            return;
        };

        let config_for_index = config.clone();
        let local_root_for_index = local_root.clone();
        let token = self.auth_token.clone();

        self.state = WizardState::Applying;
        cx.notify();

        let weak = cx.entity().downgrade();
        self._task = Some(cx.spawn(async move |_, cx| {
            let config_for_pull = config.clone();
            let local_root_for_pull = local_root.clone();
            let import_result = cx
                .background_executor()
                .spawn(async move {
                    let files = pull_markdown_files(
                        &token,
                        &config_for_pull.owner,
                        &config_for_pull.repo,
                        &config_for_pull.branch,
                        &config_for_pull.folder,
                    )?;
                    let imported_count = files.len();

                    for (remote_path, content) in files {
                        let local_path = Self::remote_to_local_path(
                            &local_root_for_pull,
                            &config_for_pull.folder,
                            &remote_path,
                        );
                        if let Some(parent) = local_path.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        std::fs::write(local_path, content)?;
                    }

                    anyhow::Result::<usize>::Ok(imported_count)
                })
                .await;

            let _ = weak.update(cx, |modal, cx| match import_result {
                Ok(imported_count) => {
                    modal.app_state.update(cx, |state, cx| {
                        state.upsert_github_binding(local_root, config, cx);
                        state.set_import_summary(
                            format!("Imported {imported_count} markdown files"),
                            cx,
                        );
                        let selected_idx = state
                            .github_bindings
                            .iter()
                            .position(|binding| {
                                binding.local_root == local_root_for_index
                                    && binding.config.owner == config_for_index.owner
                                    && binding.config.repo == config_for_index.repo
                                    && binding.config.branch == config_for_index.branch
                                    && binding.config.folder == config_for_index.folder
                            });
                        if let Some(idx) = selected_idx {
                            state.set_active_binding_idx(idx, cx);
                        }
                        state.repo_add_modal_open = false;
                        cx.notify();
                    });
                }
                Err(err) => {
                    modal.state = WizardState::Error {
                        message: format!("Failed to import markdown files: {err}"),
                    };
                    cx.notify();
                }
            });
        }));
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.app_state.update(cx, |state, cx| {
            state.repo_add_modal_open = false;
            cx.notify();
        });
    }

    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        match &self.state {
            WizardState::BranchSelect { .. } => self.load_repos(cx),
            WizardState::FolderSelect {
                repo, current_path, ..
            } => {
                if current_path.is_empty() {
                    self.select_repo(repo.clone(), cx);
                } else {
                    let parent = current_path
                        .rsplit_once('/')
                        .map(|(p, _)| p.to_string())
                        .unwrap_or_default();
                    self.enter_folder(parent, cx);
                }
            }
            WizardState::Confirm { config } => {
                let repo = RepoInfo {
                    owner: config.owner.clone(),
                    name: config.repo.clone(),
                    default_branch: config.branch.clone(),
                };
                self.select_branch(repo, config.branch.clone(), cx);
            }
            _ => {}
        }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = use_theme();
        let weak = cx.entity().downgrade();

        div()
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
                    .child(Icon::new(IconSource::Named("github".into())).size_4())
                    .child(h4("Add Repository")),
            )
            .child(
                IconButton::new(IconSource::Named("x".into()))
                    .size(px(28.0))
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _w, cx| {
                        let _ = weak.update(cx, |modal, cx| modal.close(cx));
                    }),
            )
    }

    fn render_repo_list(&self, repos: &[RepoInfo], weak: &gpui::WeakEntity<Self>) -> impl IntoElement {
        let theme = use_theme();

        div()
            .id("repo-list")
            .flex()
            .flex_col()
            .max_h(px(300.0))
            .overflow_y_scroll()
            .border_1()
            .border_color(theme.tokens.border)
            .rounded(px(8.0))
            .children(repos.iter().map(|repo| {
                let select_weak = weak.clone();
                let repo_clone = repo.clone();
                let display = format!("{}/{}", repo.owner, repo.name);

                div()
                    .px(px(12.0))
                    .py(px(10.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.tokens.accent))
                    .border_b_1()
                    .border_color(theme.tokens.border)
                    .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                        let _ = select_weak
                            .update(cx, |modal, cx| modal.select_repo(repo_clone.clone(), cx));
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(body(&display))
                            .child(
                                Icon::new(IconSource::Named("chevron-right".into()))
                                    .size_4()
                                    .color(theme.tokens.muted_foreground),
                            ),
                    )
            }))
    }

    fn render_branch_list(
        &self,
        repo: &RepoInfo,
        branches: &[BranchInfo],
        weak: &gpui::WeakEntity<Self>,
    ) -> impl IntoElement {
        let theme = use_theme();
        let repo = repo.clone();

        div()
            .id("branch-list")
            .flex()
            .flex_col()
            .max_h(px(300.0))
            .overflow_y_scroll()
            .border_1()
            .border_color(theme.tokens.border)
            .rounded(px(8.0))
            .children(branches.iter().map(|branch| {
                let select_weak = weak.clone();
                let repo_clone = repo.clone();
                let branch_name = branch.name.clone();

                div()
                    .px(px(12.0))
                    .py(px(10.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.tokens.accent))
                    .border_b_1()
                    .border_color(theme.tokens.border)
                    .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                        let _ = select_weak.update(cx, |modal, cx| {
                            modal.select_branch(repo_clone.clone(), branch_name.clone(), cx)
                        });
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(body(&branch.name))
                            .child(
                                Icon::new(IconSource::Named("chevron-right".into()))
                                    .size_4()
                                    .color(theme.tokens.muted_foreground),
                            ),
                    )
            }))
    }

    fn render_folder_list(
        &self,
        folders: &[FolderEntry],
        weak: &gpui::WeakEntity<Self>,
    ) -> impl IntoElement {
        let theme = use_theme();

        let dirs: Vec<_> = folders.iter().filter(|f| f.is_dir).collect();
        if dirs.is_empty() {
            return div()
                .py(px(24.0))
                .child(body_small("No subfolders").color(theme.tokens.muted_foreground).text_center())
                .into_any_element();
        }

        div()
            .id("folder-list")
            .flex()
            .flex_col()
            .max_h(px(200.0))
            .overflow_y_scroll()
            .border_1()
            .border_color(theme.tokens.border)
            .rounded(px(8.0))
            .children(dirs.iter().map(|folder| {
                let select_weak = weak.clone();
                let folder_path = folder.path.clone();

                div()
                    .px(px(12.0))
                    .py(px(10.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.tokens.accent))
                    .border_b_1()
                    .border_color(theme.tokens.border)
                    .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                        let _ = select_weak
                            .update(cx, |modal, cx| modal.enter_folder(folder_path.clone(), cx));
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                Icon::new(IconSource::Named("folder".into()))
                                    .size_4()
                                    .color(theme.tokens.muted_foreground),
                            )
                            .child(body(&folder.name))
                            .child(div().flex_1())
                            .child(
                                Icon::new(IconSource::Named("chevron-right".into()))
                                    .size_4()
                                    .color(theme.tokens.muted_foreground),
                            ),
                    )
            }))
            .into_any_element()
    }

    fn render_content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = use_theme();
        let weak = cx.entity().downgrade();

        match &self.state {
            WizardState::RepoSelect { repos, loading } => {
                if *loading {
                    return div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(px(200.0))
                        .child(Spinner::new().size(SpinnerSize::Md))
                        .into_any_element();
                }

                let search_weak = weak.clone();
                let query = self
                    .repo_search_input
                    .read(cx)
                    .content()
                    .trim()
                    .to_ascii_lowercase();
                let filtered_repos = if query.is_empty() {
                    repos.to_vec()
                } else {
                    repos
                        .iter()
                        .filter(|repo| {
                            let full =
                                format!("{}/{}", repo.owner, repo.name).to_ascii_lowercase();
                            full.contains(&query)
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                };

                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(body("Select a repository:"))
                    .child(
                        Input::new(&self.repo_search_input)
                            .placeholder("Search repositories...")
                            .on_change(move |_value, cx| {
                                let _ = search_weak.update(cx, |_modal, cx| cx.notify());
                            }),
                    )
                    .child(
                        body_small(&format!("{} repositories", filtered_repos.len()))
                            .color(theme.tokens.muted_foreground),
                    )
                    .child(self.render_repo_list(&filtered_repos, &weak))
                    .into_any_element()
            }
            WizardState::BranchSelect {
                repo,
                branches,
                loading,
            } => {
                if *loading {
                    return div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(px(200.0))
                        .child(Spinner::new().size(SpinnerSize::Md))
                        .into_any_element();
                }

                let back_weak = weak.clone();
                let repo_name = format!("{}/{}", repo.owner, repo.name);

                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                IconButton::new(IconSource::Named("chevron-right".into()))
                                    .size(px(28.0))
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(move |_, _w, cx| {
                                        let _ = back_weak.update(cx, |modal, cx| modal.go_back(cx));
                                    }),
                            )
                            .child(body(&repo_name)),
                    )
                    .child(body_small("Select a branch:").color(theme.tokens.muted_foreground))
                    .child(self.render_branch_list(repo, branches, &weak))
                    .into_any_element()
            }
            WizardState::FolderSelect {
                repo,
                branch,
                folders,
                current_path,
                loading,
            } => {
                if *loading {
                    return div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(px(200.0))
                        .child(Spinner::new().size(SpinnerSize::Md))
                        .into_any_element();
                }

                let back_weak = weak.clone();
                let confirm_weak = weak.clone();
                let path_display = if current_path.is_empty() {
                    format!("{}/{}:{} (root)", repo.owner, repo.name, branch)
                } else {
                    format!("{}/{}:{}/{}", repo.owner, repo.name, branch, current_path)
                };

                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                IconButton::new(IconSource::Named("chevron-right".into()))
                                    .size(px(28.0))
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(move |_, _w, cx| {
                                        let _ = back_weak.update(cx, |modal, cx| modal.go_back(cx));
                                    }),
                            )
                            .child(body_small(&path_display).color(theme.tokens.muted_foreground)),
                    )
                    .child(body("Select a folder or use current:"))
                    .child(self.render_folder_list(folders, &weak))
                    .child(
                        Button::new("confirm-folder", "Use this folder")
                            .variant(ButtonVariant::Default)
                            .on_click(move |_, _w, cx| {
                                let _ = confirm_weak.update(cx, |modal, cx| modal.confirm_folder(cx));
                            }),
                    )
                    .into_any_element()
            }
            WizardState::Confirm { config } => {
                let apply_weak = weak.clone();
                let back_weak = weak.clone();
                let choose_root_weak = weak.clone();
                let use_default_root_weak = weak.clone();
                let repo_name_for_pick = config.repo.clone();
                let repo_name_for_default = config.repo.clone();
                let suggested_default_root = default_local_root_for_repo(&config.repo)
                    .display()
                    .to_string();
                let local_root_display = self
                    .selected_local_root
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "(not selected)".to_string());
                let already_enabled = self.app_state.read(cx).github_bindings.iter().any(|binding| {
                    binding.config == *config
                        && self
                            .selected_local_root
                            .as_ref()
                            .map(|root| *root == binding.local_root)
                            .unwrap_or(false)
                });
                let path_display = if config.folder.is_empty() {
                    format!("{}/{}:{}", config.owner, config.repo, config.branch)
                } else {
                    format!(
                        "{}/{}:{}/{}",
                        config.owner, config.repo, config.branch, config.folder
                    )
                };

                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.0))
                    .py(px(16.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::new(IconSource::Named("check".into()))
                                    .size_12()
                                    .color(theme.tokens.primary),
                            ),
                    )
                    .child(
                        if already_enabled {
                            h4("Sync is enabled").text_center()
                        } else {
                            h4("Ready to sync!").text_center()
                        },
                    )
                    .child(
                        div()
                            .px(px(16.0))
                            .py(px(12.0))
                            .bg(theme.tokens.muted)
                            .rounded(px(8.0))
                            .child(body(&path_display).text_center()),
                    )
                    .child(
                        div()
                            .px(px(16.0))
                            .py(px(12.0))
                            .bg(theme.tokens.muted)
                            .rounded(px(8.0))
                            .child(
                                body_small(format!("Local folder: {}", local_root_display))
                                    .text_center(),
                            ),
                    )
                    .child(
                        body_small(format!("Suggested: {}", suggested_default_root))
                            .color(theme.tokens.muted_foreground)
                            .text_center(),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.0))
                            .child(
                                Button::new("choose-local-root", "Choose local folder")
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(move |_, _w, cx| {
                                        let _ = choose_root_weak.update(cx, |modal, cx| {
                                            modal.choose_local_root(repo_name_for_pick.clone(), cx)
                                        });
                                    }),
                            )
                            .child(
                                Button::new("create-default-local-root", "Create & use default")
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(move |_, _w, cx| {
                                        let _ = use_default_root_weak.update(cx, |modal, cx| {
                                            modal.use_default_local_root(
                                                repo_name_for_default.clone(),
                                                cx,
                                            )
                                        });
                                    }),
                            ),
                    )
                    .child(
                        body_small("Documents will be pushed to this location on every save.")
                            .color(theme.tokens.muted_foreground)
                            .text_center(),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.0))
                            .child(
                                Button::new("back", "Change")
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(move |_, _w, cx| {
                                        let _ = back_weak.update(cx, |modal, cx| modal.go_back(cx));
                                    }),
                            )
                            .child(
                                Button::new(
                                    "apply",
                                    if already_enabled {
                                        "Keep Enabled"
                                    } else {
                                        "Enable Sync"
                                    },
                                )
                                .variant(ButtonVariant::Default)
                                .on_click(move |_, _w, cx| {
                                    let _ = apply_weak.update(cx, |modal, cx| modal.apply_config(cx));
                                }),
                            ),
                    )
                    .into_any_element()
            }
            WizardState::Error { message } => {
                let retry_weak = weak.clone();

                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(16.0))
                    .py(px(32.0))
                    .child(
                        Icon::new(IconSource::Named("alert-circle".into()))
                            .size_12()
                            .color(theme.tokens.destructive),
                    )
                    .child(body(message))
                    .child(
                        Button::new("retry", "Try Again")
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _w, cx| {
                                let _ = retry_weak.update(cx, |modal, cx| modal.init(cx));
                            }),
                    )
                    .into_any_element()
            }
            WizardState::Applying => div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(12.0))
                .py(px(28.0))
                .child(Spinner::new().size(SpinnerSize::Md))
                .child(body("Importing markdown files from selected folder..."))
                .child(body_small("Only .md files are imported (including subfolders).").color(theme.tokens.muted_foreground))
                .into_any_element(),
        }
    }
}

impl Render for RepoAddModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.initialized && !self.auth_token.is_empty() {
            self.initialized = true;
            self.load_repos(cx);
        }

        let weak = cx.entity().downgrade();
        let close_weak = weak.clone();
        let close_handler = move |_w: &mut Window, cx: &mut App| {
            let _ = close_weak.update(cx, |modal, cx| modal.close(cx));
        };

        ModalDialog::new()
            .width(px(480.0))
            .on_backdrop_click(close_handler)
            .header(self.render_header(cx))
            .content(div().p(px(16.0)).child(self.render_content(cx)))
    }
}
