//! 開発・デザイン確認用のコマンドラインオプション。

/// `std::env::args()` から読み取る開発用オプション。未知の引数は無視する。
#[derive(Debug, Clone, Default)]
pub struct DevOptions {
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
