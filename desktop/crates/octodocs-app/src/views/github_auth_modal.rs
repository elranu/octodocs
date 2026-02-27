//! GitHub Auth Modal — authentication-only flow.

use adabraka_ui::components::confirm_dialog::Dialog as ModalDialog;
use adabraka_ui::prelude::*;
use gpui::{ClipboardItem, Task};
use octodocs_github::{
    get_stored_token, start_device_flow, store_token, wait_for_token, DeviceFlowHandle,
};

use crate::app_state::{AppState, PostAuthAction};

fn github_client_id() -> Option<String> {
    std::env::var("GITHUB_CLIENT_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn open_external_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("xdg-open failed: {e}"))?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("open failed: {e}"))?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map_err(|e| format!("start failed: {e}"))?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("Opening links is not supported on this OS".to_string())
}

#[derive(Debug, Clone)]
pub enum AuthState {
    Loading,
    Unauthenticated,
    DeviceFlow {
        user_code: String,
        verification_uri: String,
    },
    Error {
        message: String,
    },
}

#[allow(clippy::type_complexity)]
pub struct GithubAuthModal {
    app_state: Entity<AppState>,
    state: AuthState,
    on_authenticated: Box<dyn Fn(String, &mut Context<GithubAuthModal>)>,
    _task: Option<Task<()>>,
}

impl GithubAuthModal {
    #[allow(clippy::type_complexity)]
    pub fn new(
        app_state: Entity<AppState>,
        on_authenticated: Box<dyn Fn(String, &mut Context<GithubAuthModal>)>,
    ) -> Self {
        Self {
            app_state,
            state: AuthState::Loading,
            on_authenticated,
            _task: None,
        }
    }

    pub fn init(&mut self, cx: &mut Context<Self>) {
        self.state = AuthState::Loading;
        cx.notify();

        let weak = cx.entity().downgrade();
        self._task = Some(cx.spawn(async move |_, cx| {
            let token = get_stored_token();

            let _ = weak.update(cx, |modal, cx| match token {
                Ok(Some(token)) => {
                    (modal.on_authenticated)(token, cx);
                    modal.app_state.update(cx, |state, cx| {
                        if matches!(state.pending_post_auth_action.take(), Some(PostAuthAction::AddRepo)) {
                            state.repo_add_modal_open = true;
                        }
                        state.auth_modal_open = false;
                        cx.notify();
                    });
                }
                Ok(None) | Err(_) => {
                    modal.state = AuthState::Unauthenticated;
                    cx.notify();
                }
            });
        }));
    }

    pub fn start_auth(&mut self, cx: &mut Context<Self>) {
        let Some(client_id) = github_client_id() else {
            self.state = AuthState::Error {
                message: "GitHub OAuth is not configured. Set GITHUB_CLIENT_ID (supports .env) and restart the app.".to_string(),
            };
            cx.notify();
            return;
        };

        self.state = AuthState::Loading;
        cx.notify();

        let weak = cx.entity().downgrade();
        self._task = Some(cx.spawn(async move |_, cx| {
            let client_id_for_request = client_id.clone();
            let handle = cx
                .background_executor()
                .spawn(async move { start_device_flow(&client_id_for_request, &["repo"]) })
                .await;

            let _ = weak.update(cx, |modal, cx| match handle {
                Ok(h) => {
                    modal.state = AuthState::DeviceFlow {
                        user_code: h.user_code.clone(),
                        verification_uri: h.verification_uri.clone(),
                    };
                    cx.notify();
                    modal.poll_for_auth(client_id.clone(), h, cx);
                }
                Err(e) => {
                    modal.state = AuthState::Error {
                        message: format!("Failed to start auth: {e}"),
                    };
                    cx.notify();
                }
            });
        }));
    }

    fn poll_for_auth(&mut self, client_id: String, handle: DeviceFlowHandle, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        self._task = Some(cx.spawn(async move |_, cx| {
            let client_id_for_poll = client_id.clone();
            let result = cx
                .background_executor()
                .spawn(async move { wait_for_token(&client_id_for_poll, &handle) })
                .await;

            let _ = weak.update(cx, |modal, cx| match result {
                Ok(token) => {
                    if let Err(e) = store_token(&token) {
                        modal.state = AuthState::Error {
                            message: format!("Failed to store token: {e}"),
                        };
                        cx.notify();
                        return;
                    }
                    (modal.on_authenticated)(token, cx);
                    modal.app_state.update(cx, |state, cx| {
                        if matches!(state.pending_post_auth_action.take(), Some(PostAuthAction::AddRepo)) {
                            state.repo_add_modal_open = true;
                        }
                        state.auth_modal_open = false;
                        cx.notify();
                    });
                }
                Err(e) => {
                    modal.state = AuthState::Error {
                        message: format!("Auth failed: {e}"),
                    };
                    cx.notify();
                }
            });
        }));
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.app_state.update(cx, |state, cx| {
            state.auth_modal_open = false;
            cx.notify();
        });
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
                    .child(h4("GitHub Authentication")),
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

    fn render_content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = use_theme();
        let weak = cx.entity().downgrade();

        match &self.state {
            AuthState::Loading => div()
                .flex()
                .items_center()
                .justify_center()
                .h(px(200.0))
                .child(Spinner::new().size(SpinnerSize::Md))
                .into_any_element(),
            AuthState::Unauthenticated => div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(16.0))
                .py(px(32.0))
                .child(
                    Icon::new(IconSource::Named("github".into()))
                        .size_12()
                        .color(theme.tokens.muted_foreground),
                )
                .child(body("Connect your GitHub account"))
                .child(
                    Button::new("connect", "Connect to GitHub")
                        .variant(ButtonVariant::Default)
                        .on_click(move |_, _w, cx| {
                            let _ = weak.update(cx, |modal, cx| modal.start_auth(cx));
                        }),
                )
                .into_any_element(),
            AuthState::DeviceFlow {
                user_code,
                verification_uri,
            } => {
                let code = user_code.clone();
                let uri = verification_uri.clone();
                let uri_for_open = uri.clone();
                let code_for_copy = code.clone();
                let weak_for_open = cx.entity().downgrade();

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
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .px(px(24.0))
                            .py(px(12.0))
                            .bg(theme.tokens.muted)
                            .rounded(px(8.0))
                            .child(h2(&code))
                            .child(
                                IconButton::new(IconSource::Named("copy".into()))
                                    .size(px(28.0))
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(move |_, _w, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            code_for_copy.clone(),
                                        ));
                                    }),
                            ),
                    )
                    .child(
                        Button::new("open-verification-page", "Open verification page")
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _w, cx| {
                                let _ = weak_for_open.update(cx, |modal, cx| {
                                    if let Err(err) = open_external_url(&uri_for_open) {
                                        modal.state = AuthState::Error {
                                            message: format!("Failed to open browser: {err}"),
                                        };
                                        cx.notify();
                                    }
                                });
                            }),
                    )
                    .child(body_small(&uri).color(theme.tokens.muted_foreground))
                    .child(body_small("Waiting for authorization...").color(theme.tokens.muted_foreground))
                    .into_any_element()
            }
            AuthState::Error { message } => {
                let retry_weak = cx.entity().downgrade();
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
        }
    }
}

impl Render for GithubAuthModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let close_weak = weak.clone();
        let close_handler = move |_w: &mut Window, cx: &mut App| {
            let _ = close_weak.update(cx, |modal, cx| modal.close(cx));
        };

        ModalDialog::new()
            .width(px(460.0))
            .on_backdrop_click(close_handler)
            .header(self.render_header(cx))
            .content(div().p(px(16.0)).child(self.render_content(cx)))
    }
}
