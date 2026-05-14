use crate::{
    error::AppError,
    source::video::{
        traits::{FrameData, VideoStream},
        web_camera::WebCameraStream,
    },
};

impl VideoStream for WebCameraStream {
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
