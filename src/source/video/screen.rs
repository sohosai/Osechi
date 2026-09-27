//! 画面キャプチャ(xcap)。モニター単位で取り込む。

use std::thread;
use std::time::Duration;

use xcap::Monitor;

use super::{FEED_CAPACITY, Frame, Kind};
use crate::error::{Context, Result};
use crate::source::{Feed, Origin, Source, SourceId};

/// 取り込みの間隔(約15fps)。画面全体のキャプチャはカメラより重いので控えめにする。
const CAPTURE_INTERVAL: Duration = Duration::from_millis(66);

/// 接続されているモニターを探す。
pub(super) fn scan() -> Vec<Source<Kind>> {
    let Ok(monitors) = Monitor::all() else {
        return Vec::new();
    };
    monitors
        .into_iter()
        .filter_map(|monitor| {
            let id = monitor.id().ok()?;
            Some(Source {
                id: SourceId::new("screen", id),
                name: monitor.name().unwrap_or_else(|_| format!("Screen {id}")),
                kind: Kind::Screen(id),
                origin: Origin::Scanned,
            })
        })
        .collect()
}

/// 専用スレッドで、モニター `monitor_id` を一定間隔で取り込み続ける。
pub(super) fn open(monitor_id: u32) -> Result<Feed<Frame>> {
    Ok(Feed::spawn(FEED_CAPACITY, move |producer| {
        match find(monitor_id) {
            Ok(monitor) => {
                while producer.send(capture(&monitor)) {
                    thread::sleep(CAPTURE_INTERVAL);
                }
            }
            Err(err) => {
                producer.send(Err(err));
            }
        }
    }))
}

fn find(monitor_id: u32) -> Result<Monitor> {
    Monitor::all()
        .context("failed to enumerate monitors")?
        .into_iter()
        .find(|monitor| monitor.id().is_ok_and(|id| id == monitor_id))
        .context(format!("monitor {monitor_id} not found"))
}

fn capture(monitor: &Monitor) -> Result<Frame> {
    let image = monitor.capture_image().context("screen capture failed")?;
    Ok(image::DynamicImage::ImageRgba8(image).into_rgb8())
}
