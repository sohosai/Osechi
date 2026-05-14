use crate::{
    error::AppError,
    source::video::{
        traits::{FrameData, VideoSource},
        web_camera::WebCamera,
    },
};

impl VideoSource for WebCamera {
    fn get_frame(&mut self) -> Result<Option<FrameData>, AppError> {
        match self.rx.try_recv() {
            Ok(Ok(frame)) => Ok(Some(frame)),
            Ok(Err(e)) => Err(e),
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err(AppError::Other("Camera stream disconnected".to_string()))
            }
        }
    }
}
