//! Plain `Send` data of the chat media UI (plan chat 9.5 to 9.7): the image viewer.

use crate::data::Pixels;

/// What the image viewer shows.
#[derive(Clone, Default)]
pub struct ViewerData {
    pub open: bool,
    /// 0-based position in the message's pictures, and how many there are.
    pub index: usize,
    pub count: usize,
    pub title: String,
    /// The full-size picture once downloaded.
    pub pixels: Option<Pixels>,
    /// The download or decoding failed.
    pub failed: bool,
    /// A short line under the toolbar ("Saved to ...").
    pub status: String,
}
