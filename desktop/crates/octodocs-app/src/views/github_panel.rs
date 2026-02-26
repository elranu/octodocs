//! GitHub Setup Panel — modal for authentication and repo/branch/folder selection.

use adabraka_ui::prelude::*;
use adabraka_ui::components::confirm_dialog::Dialog as ModalDialog;
use gpui::{Task, WeakEntity};
use octodocs_github::{
    start_device_flow, wait_for_token, get_stored_token, store_token, clear_stored_token,
    list_repos, list_branches, list_folder,
    RepoInfo, BranchInfo, FolderEntry, GitHubSyncConfig, DeviceFlowHandle,
};

use crate::app_state::AppState;

/// GitHub OAuth App client ID (hardcoded for now, can be env var later).
/// This is NOT a secret for Device Flow - only client_secret is secret.
fn github_client_id() -> Option<&'static str> {
    option_env!("GITHUB_CLIENT_ID").filter(|value| !value.trim().is_empty())
}

/// Panel states for the multi-step setup flow.
#[derive(Debug, Clone)]
pub enum PanelState {
    /// Checking if we have a stored token.
    Loading,
    /// No token found — show "Connect to GitHub" button.
    Unauthenticated,
    /// Device flow started — show code and verification URL.
    DeviceFlow {
        user_code: String,
        verification_uri: String,
    },
    /// Authenticated — show repo selector.
    RepoSelect {
        repos: Vec<RepoInfo>,
        loading: bool,
    },
    /// Repo selected — show branch selector.
    BranchSelect {
        repo: RepoInfo,
        branches: Vec<BranchInfo>,
        loading: bool,
    },
    /// Branch selected — show folder selector.
    FolderSelect {
        repo: RepoInfo,
        branch: String,
        folders: Vec<FolderEntry>,
        current_path: String,
        loading: bool,
    },
    /// Configuration complete — show summary and confirm button.
    Confirm {
        config: GitHubSyncConfig,
    },
    /// Error state.
    Error {
        message: String,
    },
}

pub struct GitHubPanel {
    app_state: Entity<AppState>,
    state: PanelState,
    _task: Option<Task<()>>,
}

impl GitHubPanel {
    pub fn new(app_state: Entity<AppState>) -> Self {
        Self {
            app_state,
            state: PanelState::Loading,
            _task: None,
        }
    }

    /// Initialize the panel by checking for existing token.
    pub fn init(&mut self, cx: &mut Context<Self>) {
        self.state = PanelState::Loading;
        cx.notify();

        let weak = cx.entity().downgrade();
        self._task = Some(cx.spawn(async move |_, cx| {
            let token = get_stored_token();

            let _ = weak.update(cx, |panel, cx| {
                match token {
                    Ok(Some(_)) => panel.load_repos(cx),
                    Ok(None) | Err(_) => {
                        panel.state = PanelState::Unauthenticated;
                        cx.notify();
                    }
                }
            });
        }));
    }

    /// Start the OAuth device flow.
    pub fn start_auth(&mut self, cx: &mut Context<Self>) {
        let Some(client_id) = github_client_id() else {
            self.state = PanelState::Error {
                message: "GitHub OAuth is not configured. Set GITHUB_CLIENT_ID and rebuild the app.".to_string(),
            };
            cx.notify();
            return;
        };

        let client_id = client_id.to_string();
        self.state = PanelState::Loading;
        cx.notify();

        let weak = cx.entity().downgrade();
        self._task = Some(cx.spawn(async move |_, cx| {
            let handle = start_device_flow(&client_id, &["repo"]);

            let _ = weak.update(cx, |panel, cx| {
                match handle {
                    Ok(h) => {
                        panel.state = PanelState::DeviceFlow {
                            user_code: h.user_code.clone(),
                            verification_uri: h.verification_uri.clone(),
                        };
                        cx.notify();
                        panel.poll_for_auth(client_id, h, cx);
                    }
                    Err(e) => {
                        panel.state = PanelState::Error {
                            message: format!("Failed to start auth: {e}"),
                        };
                        cx.notify();
                    }
                }
            });
        }));
    }

    /// Poll for token after user completes device flow.
    fn poll_for_auth(&mut self, client_id: String, handle: DeviceFlowHandle, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        self._task = Some(cx.spawn(async move |_, cx| {
            let result = wait_for_token(&client_id, &handle);

            let _ = weak.update(cx, |panel, cx| {
                match result {
                    Ok(token) => {
                        if let Err(e) = store_token(&token) {
                            panel.state = PanelState::Error {
                                message: format!("Failed to store token: {e}"),
                            };
                            cx.notify();
                            return;
                        }
                        panel.load_repos(cx);
                    }
                    Err(e) => {
                        panel.state = PanelState::Error {
                            message: format!("Auth failed: {e}"),
                        };
                        cx.notify();
                    }
                }
            });
        }));
    }

    /// Load repos for authenticated user.
    fn load_repos(&mut self, cx: &mut Context<Self>) {
        self.state = PanelState::RepoSelect {
            repos: vec![],
            loading: true,
        };
        cx.notify();

        let weak = cx.entity().downgrade();
        self._task = Some(cx.spawn(async move |_, cx| {
            let token = get_stored_token().ok().flatten();
            let Some(token) = token else {
                let _ = weak.update(cx, |panel, cx| {
                    panel.state = PanelState::Unauthenticated;
                    cx.notify();
                });
                return;
            };

            let repos = list_repos(&token);

            let _ = weak.update(cx, |panel, cx| {
                match repos {
                    Ok(repos) => {
                        panel.state = PanelState::RepoSelect { repos, loading: false };
                        cx.notify();
                    }
                    Err(e) => {
                        panel.state = PanelState::Error {
                            message: format!("Failed to load repos: {e}"),
                        };
                        cx.notify();
                    }
                }
            });
        }));
    }

    /// Select a repo and load its branches.
    pub fn select_repo(&mut self, repo: RepoInfo, cx: &mut Context<Self>) {
        self.state = PanelState::BranchSelect {
            repo: repo.clone(),
            branches: vec![],
            loading: true,
        };
        cx.notify();

        let weak = cx.entity().downgrade();
        let owner = repo.owner.clone();
        let name = repo.name.clone();

        self._task = Some(cx.spawn(async move |_, cx| {
            let token = get_stored_token().ok().flatten();
            let Some(token) = token else {
                let _ = weak.update(cx, |panel, cx| {
                    panel.state = PanelState::Unauthenticated;
                    cx.notify();
                });
                return;
            };

            let branches = list_branches(&token, &owner, &name);

            let _ = weak.update(cx, |panel, cx| {
                match branches {
                    Ok(branches) => {
                        panel.state = PanelState::BranchSelect {
                            repo,
                            branches,
                            loading: false,
                        };
                        cx.notify();
                    }
                    Err(e) => {
                        panel.state = PanelState::Error {
                            message: format!("Failed to load branches: {e}"),
                        };
                        cx.notify();
                    }
                }
            });
        }));
    }

    /// Select a branch and load root folder.
    pub fn select_branch(&mut self, repo: RepoInfo, branch: String, cx: &mut Context<Self>) {
        self.state = PanelState::FolderSelect {
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
            let token = get_stored_token().ok().flatten();
            let Some(token) = token else {
                let _ = weak.update(cx, |panel, cx| {
                    panel.state = PanelState::Unauthenticated;
                    cx.notify();
                });
                return;
            };

            let folders = list_folder(&token, &owner, &name, &branch_clone, "");

            let _ = weak.update(cx, |panel, cx| {
                match folders {
                    Ok(folders) => {
                        panel.state = PanelState::FolderSelect {
                            repo,
                            branch,
                            folders,
                            current_path: String::new(),
                            loading: false,
                        };
                        cx.notify();
                    }
                    Err(e) => {
                        panel.state = PanelState::Error {
                            message: format!("Failed to load folders: {e}"),
                        };
                        cx.notify();
                    }
                }
            });
        }));
    }

    /// Navigate into a folder.
    pub fn enter_folder(&mut self, folder_path: String, cx: &mut Context<Self>) {
        let (repo, branch) = match &self.state {
            PanelState::FolderSelect { repo, branch, .. } => (repo.clone(), branch.clone()),
            _ => return,
        };

        self.state = PanelState::FolderSelect {
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

        self._task = Some(cx.spawn(async move |_, cx| {
            let token = get_stored_token().ok().flatten();
            let Some(token) = token else {
                let _ = weak.update(cx, |panel, cx| {
                    panel.state = PanelState::Unauthenticated;
                    cx.notify();
                });
                return;
            };

            let folders = list_folder(&token, &owner, &name, &branch, &folder_path);

            let _ = weak.update(cx, |panel, cx| {
                match folders {
                    Ok(folders) => {
                        panel.state = PanelState::FolderSelect {
                            repo,
                            branch,
                            folders,
                            current_path: folder_path,
                            loading: false,
                        };
                        cx.notify();
                    }
                    Err(e) => {
                        panel.state = PanelState::Error {
                            message: format!("Failed to load folders: {e}"),
                        };
                        cx.notify();
                    }
                }
            });
        }));
    }

    /// Confirm current folder selection.
    pub fn confirm_folder(&mut self, cx: &mut Context<Self>) {
        let config = match &self.state {
            PanelState::FolderSelect { repo, branch, current_path, .. } => {
                GitHubSyncConfig {
                    owner: repo.owner.clone(),
                    repo: repo.name.clone(),
                    branch: branch.clone(),
                    folder: current_path.clone(),
                }
            }
            _ => return,
        };

        self.state = PanelState::Confirm { config };
        cx.notify();
    }

    /// Apply the config to AppState and close panel.
    pub fn apply_config(&mut self, cx: &mut Context<Self>) {
        let config = match &self.state {
            PanelState::Confirm { config } => config.clone(),
            _ => return,
        };

        self.app_state.update(cx, |state, cx| {
            state.github_config = Some(config);
            state.github_panel_open = false;
            cx.notify();
        });
    }

    /// Disconnect — clear token and reset panel.
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        let _ = clear_stored_token();
        self.app_state.update(cx, |state, cx| {
            state.github_config = None;
            state.github_sync_status = octodocs_github::SyncStatus::Idle;
            cx.notify();
        });
        self.state = PanelState::Unauthenticated;
        cx.notify();
    }

    /// Close the panel.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.app_state.update(cx, |state, cx| {
            state.github_panel_open = false;
            cx.notify();
        });
    }

    /// Go back one step in the flow.
    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        match &self.state {
            PanelState::BranchSelect { .. } => self.load_repos(cx),
            PanelState::FolderSelect { repo, current_path, .. } => {
                if current_path.is_empty() {
                    // Go back to branch select
                    self.select_repo(repo.clone(), cx);
                } else {
                    // Go up one directory
                    let parent = current_path
                        .rsplit_once('/')
                        .map(|(p, _)| p.to_string())
                        .unwrap_or_default();
                    self.enter_folder(parent, cx);
                }
            }
            PanelState::Confirm { config } => {
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
}

impl Render for GitHubPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = use_theme();
        let weak = cx.entity().downgrade();

        // Close handler for backdrop click
        let close_weak = weak.clone();
        let close_handler = move |_w: &mut Window, cx: &mut App| {
            let _ = close_weak.update(cx, |panel, cx| panel.close(cx));
        };

        let content = self.render_content(&weak, cx);

        ModalDialog::new()
            .width(px(480.0))
            .on_backdrop_click(close_handler)
            .header(self.render_header(&weak, cx))
            .content(
                div()
                    .p(px(16.0))
                    .child(content)
            )
    }
}

impl GitHubPanel {
    fn render_header(&self, weak: &WeakEntity<Self>, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = use_theme();
        let close_weak = weak.clone();

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
                    .child(h4("GitHub Sync"))
            )
            .child(
                IconButton::new(IconSource::Named("x".into()))
                    .size(px(28.0))
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _w, cx| {
                        let _ = close_weak.update(cx, |panel, cx| panel.close(cx));
                    })
            )
    }

    fn render_content(&mut self, weak: &WeakEntity<Self>, cx: &mut Context<Self>) -> AnyElement {
        let theme = use_theme();

        match &self.state {
            PanelState::Loading => {
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(px(200.0))
                    .child(Spinner::new().size(SpinnerSize::Md))
                    .into_any_element()
            }

            PanelState::Unauthenticated => {
                let auth_weak = weak.clone();
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(16.0))
                    .py(px(32.0))
                    .child(Icon::new(IconSource::Named("github".into())).size_12().color(theme.tokens.muted_foreground))
                    .child(body("Connect your GitHub account to sync documents"))
                    .child(
                        Button::new("connect", "Connect to GitHub")
                            .variant(ButtonVariant::Default)
                            .on_click(move |_, _w, cx| {
                                let _ = auth_weak.update(cx, |panel, cx| panel.start_auth(cx));
                            })
                    )
                    .into_any_element()
            }

            PanelState::DeviceFlow { user_code, verification_uri } => {
                let code = user_code.clone();
                let uri = verification_uri.clone();

                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(16.0))
                    .py(px(24.0))
                    .child(Spinner::new().size(SpinnerSize::Md))
                    .child(body("Enter this code on GitHub:"))
                    .child(
                        div()
                            .px(px(24.0))
                            .py(px(12.0))
                            .bg(theme.tokens.muted)
                            .rounded(px(8.0))
                            .child(h2(&code))
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(body_small(&uri))
                            .child(Icon::new(IconSource::Named("external-link".into())).size_3())
                    )
                    .child(body_small("Waiting for authorization...").color(theme.tokens.muted_foreground))
                    .into_any_element()
            }

            PanelState::RepoSelect { repos, loading } => {
                if *loading {
                    return div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(px(200.0))
                        .child(Spinner::new().size(SpinnerSize::Md))
                        .into_any_element();
                }

                let disconnect_weak = weak.clone();

                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(body("Select a repository:"))
                            .child(
                                Button::new("disconnect", "Disconnect")
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .on_click(move |_, _w, cx| {
                                        let _ = disconnect_weak.update(cx, |panel, cx| panel.disconnect(cx));
                                    })
                            )
                    )
                    .child(self.render_repo_list(repos, weak))
                    .into_any_element()
            }

            PanelState::BranchSelect { repo, branches, loading } => {
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
                                        let _ = back_weak.update(cx, |panel, cx| panel.go_back(cx));
                                    })
                            )
                            .child(body(&repo_name))
                    )
                    .child(body_small("Select a branch:").color(theme.tokens.muted_foreground))
                    .child(self.render_branch_list(repo, branches, weak))
                    .into_any_element()
            }

            PanelState::FolderSelect { repo, branch, folders, current_path, loading } => {
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
                                        let _ = back_weak.update(cx, |panel, cx| panel.go_back(cx));
                                    })
                            )
                            .child(body_small(&path_display).color(theme.tokens.muted_foreground))
                    )
                    .child(body("Select a folder or use current:"))
                    .child(self.render_folder_list(repo, branch, folders, weak))
                    .child(
                        Button::new("confirm-folder", "Use this folder")
                            .variant(ButtonVariant::Default)
                            .on_click(move |_, _w, cx| {
                                let _ = confirm_weak.update(cx, |panel, cx| panel.confirm_folder(cx));
                            })
                    )
                    .into_any_element()
            }

            PanelState::Confirm { config } => {
                let apply_weak = weak.clone();
                let back_weak = weak.clone();
                let path_display = if config.folder.is_empty() {
                    format!("{}/{}:{}", config.owner, config.repo, config.branch)
                } else {
                    format!("{}/{}:{}/{}", config.owner, config.repo, config.branch, config.folder)
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
                            .child(Icon::new(IconSource::Named("check".into())).size_12().color(theme.tokens.primary))
                    )
                    .child(h4("Ready to sync!").text_center())
                    .child(
                        div()
                            .px(px(16.0))
                            .py(px(12.0))
                            .bg(theme.tokens.muted)
                            .rounded(px(8.0))
                            .child(body(&path_display).text_center())
                    )
                    .child(body_small("Documents will be pushed to this location on every save.").color(theme.tokens.muted_foreground).text_center())
                    .child(
                        div()
                            .flex()
                            .gap(px(8.0))
                            .child(
                                Button::new("back", "Change")
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(move |_, _w, cx| {
                                        let _ = back_weak.update(cx, |panel, cx| panel.go_back(cx));
                                    })
                            )
                            .child(
                                Button::new("apply", "Enable Sync")
                                    .variant(ButtonVariant::Default)
                                    .on_click(move |_, _w, cx| {
                                        let _ = apply_weak.update(cx, |panel, cx| panel.apply_config(cx));
                                    })
                            )
                    )
                    .into_any_element()
            }

            PanelState::Error { message } => {
                let retry_weak = weak.clone();

                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(16.0))
                    .py(px(32.0))
                    .child(Icon::new(IconSource::Named("alert-circle".into())).size_12().color(theme.tokens.destructive))
                    .child(body(message))
                    .child(
                        Button::new("retry", "Try Again")
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _w, cx| {
                                let _ = retry_weak.update(cx, |panel, cx| panel.init(cx));
                            })
                    )
                    .into_any_element()
            }
        }
    }

    fn render_repo_list(&self, repos: &[RepoInfo], weak: &WeakEntity<Self>) -> impl IntoElement {
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
                        let _ = select_weak.update(cx, |panel, cx| panel.select_repo(repo_clone.clone(), cx));
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(body(&display))
                            .child(Icon::new(IconSource::Named("chevron-right".into())).size_4().color(theme.tokens.muted_foreground))
                    )
            }))
    }

    fn render_branch_list(&self, repo: &RepoInfo, branches: &[BranchInfo], weak: &WeakEntity<Self>) -> impl IntoElement {
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
                        let _ = select_weak.update(cx, |panel, cx| panel.select_branch(repo_clone.clone(), branch_name.clone(), cx));
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(body(&branch.name))
                            .child(Icon::new(IconSource::Named("chevron-right".into())).size_4().color(theme.tokens.muted_foreground))
                    )
            }))
    }

    fn render_folder_list(&self, repo: &RepoInfo, branch: &str, folders: &[FolderEntry], weak: &WeakEntity<Self>) -> impl IntoElement {
        let theme = use_theme();
        let repo = repo.clone();
        let branch = branch.to_string();

        // Filter to only show directories
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
                        let _ = select_weak.update(cx, |panel, cx| panel.enter_folder(folder_path.clone(), cx));
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(Icon::new(IconSource::Named("folder".into())).size_4().color(theme.tokens.muted_foreground))
                            .child(body(&folder.name))
                            .child(div().flex_1())
                            .child(Icon::new(IconSource::Named("chevron-right".into())).size_4().color(theme.tokens.muted_foreground))
                    )
            }))
            .into_any_element()
    }
}
