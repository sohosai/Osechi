use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use xcap::Monitor;

use crate::error::AppError;
use crate::source::video::{FrameData, Stream};

/// 1秒間に取得を試みるフレーム数の上限。画面全体のキャプチャは
/// カメラ映像より重いため、WEBカメラより控えめな値にしている。
const CAPTURE_INTERVAL: Duration = Duration::from_millis(66);

/// 画面共有(モニターキャプチャ)のアクティブなストリーム。
/// バックグラウンドスレッドで一定間隔ごとに画面を取得し、
/// チャネル経由でメインスレッドに送信する。
pub struct ScreenCaptureStream {
    pub monitor_id: u32,
    pub rx: Option<mpsc::Receiver<Result<FrameData, AppError>>>,
}

impl ScreenCaptureStream {
    pub fn new(monitor_id: u32) -> Self {
        Self {
            monitor_id,
            rx: None,
        }
    }

    /// キャプチャを開始する。バックグラウンドスレッドを起動し、
    /// 対象モニターの取得を繰り返す。
    pub fn open_stream(&mut self) -> Result<(), AppError> {
        let (tx, rx) = mpsc::sync_channel(2);
        let monitor_id = self.monitor_id;

        thread::spawn(move || {
            let monitor = match find_monitor(monitor_id) {
                Ok(monitor) => monitor,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };

            loop {
                let frame = capture_frame(&monitor);
                if tx.send(frame).is_err() {
                    break;
                }
                thread::sleep(CAPTURE_INTERVAL);
            }
        });

        self.rx = Some(rx);
        Ok(())
    }
}

impl Stream for ScreenCaptureStream {
    fn get_frame(&mut self) -> Result<Option<FrameData>, AppError> {
        if self.rx.is_none() {
            self.open_stream()?;
        }

        match self.rx.as_mut().unwrap().try_recv() {
            Ok(Ok(frame)) => Ok(Some(frame)),
            Ok(Err(e)) => Err(e),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Err(AppError::Other(
                "Screen capture stream disconnected".to_string(),
            )),
        }
    }
}

fn find_monitor(monitor_id: u32) -> Result<Monitor, AppError> {
    Monitor::all()
        .map_err(|e| AppError::Other(format!("Failed to enumerate monitors: {}", e)))?
        .into_iter()
        .find(|m| m.id().map(|id| id == monitor_id).unwrap_or(false))
        .ok_or_else(|| AppError::Other(format!("Monitor {} not found", monitor_id)))
}

fn capture_frame(monitor: &Monitor) -> Result<FrameData, AppError> {
    let image = monitor
        .capture_image()
        .map_err(|e| AppError::Other(format!("Screen capture failed: {}", e)))?;

    let width = image.width();
    let height = image.height();

    // RGBA -> RGB (FrameData はタイトパックの RGB8 を前提とする)
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
    for px in image.into_raw().chunks_exact(4) {
        pixels.extend_from_slice(&px[0..3]);
    }

    Ok(FrameData {
        pixels: Arc::new(pixels),
        width,
        height,
    })
}
