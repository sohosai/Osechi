use crate::{
    error::AppError,
    source::video::{
        traits::{FrameData, VideoSource},
        web_camera::WebCamera,
    },
};

impl VideoSource for WebCamera {
    fn get_frame(&mut self) -> Result<Option<FrameData>, AppError> {
        let mut latest_frame = None;
        let mut last_error = None;
        while let Ok(result) = self.rx.try_recv() {
            match result {
                Ok(frame) => latest_frame = Some(frame),
                Err(e) => last_error = Some(e),
            }
        }

        if let Some(e) = last_error {
            return Err(e);
        }

        Ok(latest_frame)
    }
}
