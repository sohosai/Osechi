pub mod impls;

use std::sync::{Arc, mpsc};
use std::thread;

use nokhwa::Camera;
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};

use crate::error::AppError;
use crate::source::video::traits::{FrameData, VideoSourceId};

pub struct WebCamera {
    pub id: VideoSourceId,
    pub name: String,
    pub index: CameraIndex,
    pub rx: Option<mpsc::Receiver<Result<FrameData, AppError>>>,
}

impl WebCamera {
    pub fn new(id: VideoSourceId, name: String, index: CameraIndex) -> Self {
        Self {
            id,
            name,
            index,
            rx: None,
        }
    }

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
                                    pixels: Arc::new(pixels),
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
