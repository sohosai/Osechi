//! 起動時の設定。コマンドライン引数と、会場固有の既定値(常設のAES67フローなど)をまとめる。

use std::net::Ipv4Addr;

use crate::source::audio::aes67;

/// 会場のYamaha DM7が送出するAES67フロー(Dante Controllerでマルチキャストフロー作成済み)。
const DM7: aes67::Config = aes67::Config {
    addr: Ipv4Addr::new(239, 69, 123, 1),
    port: 5004,
    payload_type: 96,
    format: aes67::Format::L24,
    channels: 1,
    sample_rate: 48_000,
};

#[derive(Debug, Clone)]
pub struct Config {
    /// ウインドウの初期サイズ(内寸)。`--window-size WxH` で上書きする。
    pub window_size: [f32; 2],
    /// 起動直後に最初の音声入力デバイスをミキサーへ追加する(`--demo-mixer`)。
    /// ドラッグ操作なしでミキサーの見た目を確認するための開発用オプション。
    pub demo_mixer: bool,
    /// 起動時にソース一覧とミキサーへ追加するAES67フロー(表示名, 設定)。
    pub aes67_flows: Vec<(String, aes67::Config)>,
    /// 配信パネルに最初から入れておくRTMPの送出先(`--rtmp-url rtmp://host/app/stream`)。
    pub rtmp_url: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            window_size: [1280.0, 760.0],
            demo_mixer: false,
            aes67_flows: vec![("DM7 AES67".to_string(), DM7)],
            rtmp_url: None,
        }
    }
}

impl Config {
    /// プログラム名を除いた引数から設定を作る。未知の引数・不正な値は無視する。
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Self {
        let mut config = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--demo-mixer" => config.demo_mixer = true,
                "--rtmp-url" => config.rtmp_url = args.next(),
                "--window-size" => {
                    if let Some(size) = args.next().as_deref().and_then(parse_size) {
                        config.window_size = size;
                    }
                }
                _ => {}
            }
        }
        config
    }
}

/// `1280x720` を `[1280.0, 720.0]` にする。
fn parse_size(spec: &str) -> Option<[f32; 2]> {
    let (width, height) = spec.split_once('x')?;
    Some([width.trim().parse().ok()?, height.trim().parse().ok()?])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn defaults_include_dm7_flow() {
        let config = Config::from_args(args(&[]));
        assert!(!config.demo_mixer);
        assert_eq!(config.window_size, [1280.0, 760.0]);
        assert_eq!(config.aes67_flows, [("DM7 AES67".to_string(), DM7)]);
    }

    #[test]
    fn parses_flags() {
        let config = Config::from_args(args(&[
            "--demo-mixer",
            "--window-size",
            "800x600",
            "--rtmp-url",
            "rtmp://127.0.0.1/live/1A",
        ]));
        assert!(config.demo_mixer);
        assert_eq!(config.window_size, [800.0, 600.0]);
        assert_eq!(config.rtmp_url.as_deref(), Some("rtmp://127.0.0.1/live/1A"));
    }

    #[test]
    fn ignores_unknown_and_invalid_arguments() {
        let config = Config::from_args(args(&["--nope", "--window-size", "big"]));
        assert_eq!(config.window_size, [1280.0, 760.0]);
    }
}
