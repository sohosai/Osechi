//! 開発・デザイン確認用のコマンドラインオプション。
//!
//! 通常の利用では使わない。UIの見た目を確認するたびに外部ツールで
//! ウインドウを操作してスクリーンショットを撮るのは手間な上、
//! ウインドウのフォーカスが奪われると無関係な画面を撮ってしまう事故もある。
//! `--screenshot` はegui自身のフレームバッファをそのまま画像化するため、
//! ウインドウが最前面か・フォーカスがあるかに一切依存しない。

use std::path::PathBuf;

use eframe::egui;

/// `std::env::args()` から読み取る開発用オプション。未知の引数は無視する。
#[derive(Debug, Clone, Default)]
pub struct DevOptions {
    /// 指定フレーム数の描画後にウインドウ内容をPNGとして保存し、アプリを終了する。
    pub screenshot_path: Option<PathBuf>,
    /// 起動直後に、検出済みの最初の音声入力デバイスをオーディオミキサーへ
    /// 自動追加する(ドラッグ操作なしでミキサーの見た目を確認するため)。
    pub demo_mixer: bool,
    /// 起動時のウインドウサイズ(内寸)を上書きする。
    pub window_size: Option<(f32, f32)>,
}

impl DevOptions {
    pub fn from_args() -> Self {
        let mut options = Self::default();
        let mut args = std::env::args().skip(1);

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--screenshot" => {
                    options.screenshot_path = args.next().map(PathBuf::from);
                }
                "--demo-mixer" => options.demo_mixer = true,
                "--window-size" => {
                    if let Some(spec) = args.next() {
                        options.window_size = parse_window_size(&spec);
                    }
                }
                _ => {}
            }
        }

        options
    }
}

fn parse_window_size(spec: &str) -> Option<(f32, f32)> {
    let (w, h) = spec.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

/// フレーム数(レイアウトが安定するまで数フレーム待つ)を数え、指定フレームに
/// 達したらスクリーンショットを要求する。要求への応答(`Event::Screenshot`)を
/// 受け取ったら画像を保存し、アプリを終了する。
///
/// `OsechiApp::ui` の中から毎フレーム呼ばれることを想定している。
pub struct ScreenshotRequester {
    path: PathBuf,
    frames_remaining: u32,
    requested: bool,
}

impl ScreenshotRequester {
    /// レイアウトが安定するまで待つフレーム数。
    const SETTLE_FRAMES: u32 = 12;

    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            frames_remaining: Self::SETTLE_FRAMES,
            requested: false,
        }
    }

    pub fn tick(&mut self, ctx: &egui::Context) {
        if !self.requested {
            if self.frames_remaining == 0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                self.requested = true;
            } else {
                self.frames_remaining -= 1;
            }
        }

        // `ctx.input()` はロックを保持したままクロージャを呼ぶため、その中で
        // `send_viewport_cmd` を呼ぶとデッドロックし得る。画像だけ取り出して
        // ロックを抜けてから保存・終了処理を行う。
        let screenshot = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });

        if let Some(image) = screenshot {
            match save_color_image(&image, &self.path) {
                Ok(()) => tracing::info!("screenshot saved to {}", self.path.display()),
                Err(e) => tracing::error!("failed to save screenshot: {e}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// `egui::ColorImage`(RGBA8のピクセル列)をPNGとして保存する。
fn save_color_image(color_image: &egui::ColorImage, path: &std::path::Path) -> Result<(), String> {
    let [width, height] = color_image.size;
    let mut bytes = Vec::with_capacity(width * height * 4);
    for pixel in &color_image.pixels {
        bytes.extend_from_slice(&[pixel.r(), pixel.g(), pixel.b(), pixel.a()]);
    }

    image::save_buffer(
        path,
        &bytes,
        width as u32,
        height as u32,
        image::ColorType::Rgba8,
    )
    .map_err(|e| e.to_string())
}
