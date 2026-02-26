mod app_state;
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
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    Application::new()
        .with_assets(Assets {
            base: manifest_dir,
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

                    cx.new(|cx| RootView::new(cx, initial_is_dark))
                },
            )
            .unwrap();
        });
}
