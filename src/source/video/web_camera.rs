use std::sync::mpsc;
use std::thread;

use nokhwa::Camera;
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};

use crate::error::AppError;
use crate::source::video::{FrameData, Stream};

/// WEBカメラのアクティブなストリーム。
/// バックグラウンドスレッドでカメラからフレームを取得し、
/// チャネル経由でメインスレッドに送信する。
pub struct WebCameraStream {
    pub index: CameraIndex,
    pub rx: Option<mpsc::Receiver<Result<FrameData, AppError>>>,
}

impl WebCameraStream {
    pub fn new(index: CameraIndex) -> Self {
        Self { index, rx: None }
    }

    /// カメラのストリームを開始する。
    /// バックグラウンドスレッドを起動し、フレームのキャプチャを開始する。
    pub fn open_stream(&mut self) -> Result<(), AppError> {
        let (tx, rx) = mpsc::sync_channel(2);
        let index = self.index.clone();

        thread::spawn(move || {
            let resolution = nokhwa::utils::Resolution::new(1280, 720);
            let requested = RequestedFormat::new::<RgbFormat>(
                RequestedFormatType::HighestResolution(resolution),
            );

            let mut cam = match Camera::new(index, requested) {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(Err(AppError::Nokhawa(e)));
                    return;
                }
            };

            if let Err(e) = cam.open_stream() {
                let _ = tx.send(Err(AppError::Nokhawa(e)));
                return;
            }

            loop {
                let frame_res = cam.frame();
                let processed = match frame_res {
                    Ok(frame) => match frame.decode_image::<RgbFormat>() {
                        Ok(rgb_image) => {
                            let width = rgb_image.width();
                            let height = rgb_image.height();
                            let pixels = rgb_image.into_raw();

                            if width == 0 || height == 0 {
                                Err(AppError::Other(format!(
                                    "Invalid resolution: {}x{}",
                                    width, height
                                )))
                            } else {
                                Ok(FrameData {
                                    pixels: std::sync::Arc::new(pixels),
                                    width,
                                    height,
                                })
                            }
                        }
                        Err(e) => Err(AppError::Nokhawa(e)),
                    },
                    Err(e) => Err(AppError::Nokhawa(e)),
                };

                if tx.send(processed).is_err() {
                    break;
                }
            }
        });

        self.rx = Some(rx);
        Ok(())
    }
}

impl Stream for WebCameraStream {
    fn get_frame(&mut self) -> Result<Option<FrameData>, AppError> {
        if self.rx.is_none() {
            self.open_stream()?;
        }

        match self.rx.as_mut().unwrap().try_recv() {
            Ok(Ok(frame)) => Ok(Some(frame)),
            Ok(Err(e)) => Err(e),
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err(AppError::Other("Camera stream disconnected".to_string()))
            }
        }
    }
}
