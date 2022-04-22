extern crate dreammaker as dm;

#[derive(Debug)]
pub enum RenderError {
    Io(std::io::Error),
    Dmm(dm::DMError),
    Octocrab(octocrab::Error),
    Other(String),
    Minimap,
}

// there has to be a better way to do this
impl From<std::io::Error> for RenderError {
    fn from(err: std::io::Error) -> Self {
        RenderError::Io(err)
    }
}

impl From<dm::DMError> for RenderError {
    fn from(err: dm::DMError) -> Self {
        RenderError::Dmm(err)
    }
}

impl From<octocrab::Error> for RenderError {
    fn from(err: octocrab::Error) -> Self {
        RenderError::Octocrab(err)
    }
}
