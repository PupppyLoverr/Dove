//! The comparison scenes' application, with the one font file every
//! framework's adapter loads (comparisons/fonts, staged by the runner as
//! `ZGX_FONT_FILE`; zgx_workload::font_file reports it).
use zgx_desktop::{Application, FontData};

pub fn application() -> Result<Application, Box<dyn std::error::Error>> {
    let mut application = Application::new();
    if let Some(bytes) = zgx_workload::font_file() {
        application = application.font(FontData::new(std::sync::Arc::<[u8]>::from(bytes))?);
    }
    Ok(application)
}
