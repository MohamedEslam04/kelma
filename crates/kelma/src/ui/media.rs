//! UI-thread side of the chat media (plan chat 9.5): fills the `MediaStore` global (the viewer).

use super::{UiHandle, img};
use crate::media_data::ViewerData;
use crate::{AppWindow, MediaStore};
use slint::ComponentHandle;

impl UiHandle {
    /// The picture viewer: open or closed, which picture, and the full-size image once it is there.
    pub fn viewer(&self, v: ViewerData) {
        self.run(move |w: AppWindow| {
            let s = w.global::<MediaStore>();
            let (image, loaded) = img(&v.pixels);
            s.set_viewer_open(v.open);
            s.set_viewer_index(v.index as i32);
            s.set_viewer_count(v.count as i32);
            s.set_viewer_title(v.title.into());
            s.set_viewer_image(image);
            s.set_viewer_loaded(loaded);
            s.set_viewer_failed(v.failed);
            s.set_viewer_status(v.status.into());
        });
    }
}
