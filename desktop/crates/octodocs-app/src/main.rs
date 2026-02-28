mod app_state;
mod updater;
mod views;

use adabraka_ui::prelude::*;
use gpui::*;
use views::root::RootView;

struct Assets {
    base: std::path::PathBuf,
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        let full = self.base.join(path);
        if full.exists() {
            std::fs::read(full)
                .map(|data| Some(std::borrow::Cow::Owned(data)))
                .map_err(|e| e.into())
        } else {
            Ok(None)
        }
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let dir = self.base.join(path);
        if !dir.is_dir() {
            return Ok(vec![]);
        }
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|e| {
                        e.ok().and_then(|entry| {
                            entry
                                .file_name()
                                .to_str()
                                .map(|s| SharedString::from(s.to_string()))
                        })
                    })
                    .collect()
            })
            .map_err(|e| e.into())
    }
}

fn main() {
    let _ = dotenvy::dotenv();

    // In production installs, assets/ lives next to the executable.
    // In development (cargo run / cargo build), the exe is inside target/{debug,release}/
    // and assets/ is not there — so fall back to CARGO_MANIFEST_DIR which always has it.
    let assets_base = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .filter(|d| d.join("assets").exists())
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")));

    Application::new()
        .with_assets(Assets {
            base: assets_base,
        })
        .run(|cx: &mut App| {
            adabraka_ui::init(cx);
            adabraka_ui::set_icon_base_path("assets/icons");

            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("OctoDocs".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let initial_is_dark = matches!(
                        window.appearance(),
                        WindowAppearance::Dark | WindowAppearance::VibrantDark
                    );

                    if initial_is_dark {
                        install_theme(cx, Theme::dark());
                    } else {
                        install_theme(cx, Theme::light());
                    }

                    let root = cx.new(|cx| RootView::new(cx, initial_is_dark));

                    // When the user tries to close the window while there are unsaved
                    // changes, surface the in-app modal instead of a blocking rfd dialog.
                    let app_state = root.read(cx).app_state.clone();
                    window.on_window_should_close(cx, move |_window, cx| {
                        let dirty = app_state.read(cx).dirty;
                        if !dirty {
                            return true;
                        }
                        // Set the pending-close flag and open the existing unsaved-prompt
                        // modal. The modal's Save/Discard buttons call cx.quit() when
                        // pending_window_close is set.
                        app_state.update(cx, |state, cx| {
                            state.pending_window_close = true;
                            state.show_unsaved_prompt = true;
                            cx.notify();
                        });
                        false // keep the window open until the user responds
                    });

                    root
                },
            )
            .unwrap();
        });
}
