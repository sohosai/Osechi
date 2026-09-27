//! Webカメラ(nokhwa)。

use std::sync::Once;

use nokhwa::Camera;
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{ApiBackend, CameraIndex, RequestedFormat, RequestedFormatType, Resolution};

use super::{FEED_CAPACITY, Frame, Kind};
use crate::error::{Context, Error, Result};
use crate::source::{Feed, Origin, Source, SourceId};

/// 接続されているWebカメラを探す。
pub(super) fn scan() -> Vec<Source<Kind>> {
    static INIT: Once = Once::new();
    INIT.call_once(|| nokhwa::nokhwa_initialize(|_| {}));

    let backend = nokhwa::native_api_backend().unwrap_or(ApiBackend::Auto);
    nokhwa::query(backend)
        .unwrap_or_default()
        .into_iter()
        .map(|info| Source {
            id: SourceId::new(
                "camera",
                format_args!("{}_{}", info.human_name(), info.index()),
            ),
            name: info.human_name().to_string(),
            kind: Kind::Camera(info.index().clone()),
            origin: Origin::Scanned,
        })
        .collect()
}

/// カメラを開き、専用スレッドでフレームを取り込み続ける。
pub(super) fn open(index: CameraIndex) -> Result<Feed<Frame>> {
    Ok(Feed::spawn(FEED_CAPACITY, move |producer| {
        match connect(index) {
            Ok(mut camera) => while producer.send(read_frame(&mut camera)) {},
            Err(err) => {
                producer.send(Err(err));
            }
        }
    }))
}

fn connect(index: CameraIndex) -> Result<Camera> {
    let format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::HighestResolution(
        Resolution::new(1280, 720),
    ));
    let mut camera = Camera::new(index, format).context("failed to open camera")?;
    camera
        .open_stream()
        .context("failed to start camera stream")?;
    Ok(camera)
}

fn read_frame(camera: &mut Camera) -> Result<Frame> {
    let frame = camera
        .frame()
        .context("failed to read camera frame")?
        .decode_image::<RgbFormat>()
        .context("failed to decode camera frame")?;
    if frame.width() == 0 || frame.height() == 0 {
        return Err(Error::new(format!(
            "invalid resolution: {}x{}",
            frame.width(),
            frame.height()
        )));
    }
    Ok(frame)
}
