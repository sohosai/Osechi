use crate::{
    error::AppError,
    source::video::{
        traits::{FrameData, VideoSource, VideoSourceId},
        web_camera::WebCamera,
    },
};

impl VideoSource for WebCamera {
    fn id(&self) -> VideoSourceId {
        self.id.clone()
    }

    fn name(&self) -> String {
        self.name.clone()
    }

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

    fn clone_unopened(&self) -> Box<dyn VideoSource> {
        Box::new(Self {
            id: self.id.clone(),
            name: self.name.clone(),
            index: self.index.clone(),
            rx: None,
        })
    }
}
