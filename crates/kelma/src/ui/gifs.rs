//! UI-thread side of the GIF picker: fills the `GifStore` global.

use super::{UiHandle, img, model};
use crate::backend::GifRow;
use crate::{GifItem, GifStore};
use slint::ComponentHandle;

impl UiHandle {
    /// The picker's results (previews as they arrive) and a status line.
    pub fn gifs(&self, rows: Vec<GifRow>, status: String) {
        self.run(move |w| {
            let s = w.global::<GifStore>();
            s.set_items(model(
                rows.into_iter()
                    .map(|r| {
                        let (preview, has_preview) = img(&r.preview);
                        GifItem {
                            title: r.title.into(),
                            preview,
                            has_preview,
                        }
                    })
                    .collect(),
            ));
            s.set_status(status.into());
        });
    }
}
