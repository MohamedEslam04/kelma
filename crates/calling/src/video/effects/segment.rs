//! Where the person is: MediaPipe's selfie segmentation (the landscape model, 256x144), run on
//! the CPU by tract.
//!
//! The model is Google's `selfie_segmentation_landscape.tflite` (Apache-2.0), converted to ONNX
//! by `assets/segmentation/convert.py` (its output matches MediaPipe's own to 2e-4); the licence
//! and where it came from are next to it. It is about 30 small convolutions: a few milliseconds a
//! frame on a laptop core.

use super::pixels::{Mask, model_input};
use crate::video::I420Frame;
use crate::{Error, Result};
use tract_onnx::prelude::{
    Framework, InferenceModelExt, IntoRunnable, Tensor, TypedRunnableModel, tvec,
};

/// The model's input and output size: 256x144 (16:9).
pub const MODEL_SIZE: (usize, usize) = (256, 144);

static MODEL: &[u8] = include_bytes!("../../../assets/segmentation/selfie_segmentation.onnx");

/// The loaded model, ready to run.
pub struct Segmenter {
    plan: std::sync::Arc<TypedRunnableModel>,
}

impl Segmenter {
    /// Loads and optimizes the bundled model (tens of milliseconds; once per camera).
    pub fn new() -> Result<Self> {
        let plan = tract_onnx::onnx()
            .model_for_read(&mut std::io::Cursor::new(MODEL))
            .and_then(|m| m.into_optimized())
            .and_then(|m| m.into_runnable())
            .map_err(|e| Error::Media(format!("segmentation model: {e:#}")))?;
        Ok(Segmenter { plan })
    }

    /// The person probability of each pixel of `rgb` (planar RGB in 0..1, [`MODEL_SIZE`]).
    pub fn probabilities(&self, rgb: &[f32]) -> Result<Vec<f32>> {
        let (w, h) = MODEL_SIZE;
        let fail =
            |e: tract_onnx::prelude::TractError| Error::Media(format!("segmentation: {e:#}"));
        let input = Tensor::from_shape(&[1, 3, h, w], rgb).map_err(fail)?;
        let out = self.plan.run(tvec!(input.into())).map_err(fail)?;
        let first = out
            .first()
            .ok_or_else(|| Error::Media("segmentation: no output".into()))?;
        let p = first
            .try_as_plain_ram()
            .and_then(|v| v.as_slice::<f32>().map(<[f32]>::to_vec))
            .map_err(fail)?;
        Ok(p)
    }

    /// Where the person is in `frame`, at [`MODEL_SIZE`].
    pub fn mask(&self, frame: &I420Frame) -> Result<Mask> {
        let (w, h) = MODEL_SIZE;
        let p = self.probabilities(&model_input(frame, w, h))?;
        Ok(Mask::from_probabilities(&p, w, h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A drawn head and shoulders on a plain wall, as planar RGB at the model's size: no photo
    /// of anyone in the repository, and still enough of a person for the model.
    fn cartoon() -> Vec<f32> {
        let (w, h) = MODEL_SIZE;
        let mut px = vec![[0.75f32, 0.78, 0.80]; w * h];
        let ellipse = |x: f32, y: f32, cx: f32, cy: f32, rx: f32, ry: f32| {
            ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) < 1.0
        };
        for (i, p) in px.iter_mut().enumerate() {
            let (x, y) = ((i % w) as f32, (i / w) as f32);
            if ellipse(x, y, 128.0, 150.0, 70.0, 55.0) {
                *p = [0.20, 0.30, 0.55];
            }
            if (x - 128.0).abs() < 12.0 && y > 70.0 && y < 100.0 {
                *p = [0.85, 0.65, 0.52];
            }
            if ellipse(x, y, 128.0, 52.0, 30.0, 36.0) {
                *p = [0.15, 0.10, 0.08];
            }
            if ellipse(x, y, 128.0, 60.0, 25.0, 31.0) && y > 36.0 {
                *p = [0.88, 0.68, 0.55];
            }
            for ex in [119.0, 137.0] {
                if ellipse(x, y, ex, 56.0, 3.5, 2.2) {
                    *p = [0.1, 0.1, 0.1];
                }
            }
            if (x - 128.0).abs() < 7.0 && (y - 74.0).abs() < 1.2 {
                *p = [0.6, 0.3, 0.3];
            }
        }
        let mut planar = vec![0f32; 3 * w * h];
        for (i, p) in px.iter().enumerate() {
            for (c, v) in p.iter().enumerate() {
                planar[c * w * h + i] = *v;
            }
        }
        planar
    }

    #[test]
    fn the_bundled_model_finds_the_person_and_not_the_wall() {
        let s = Segmenter::new().unwrap();
        let p = s.probabilities(&cartoon()).unwrap();
        let (w, h) = MODEL_SIZE;
        assert_eq!(p.len(), w * h);
        let at = |x: usize, y: usize| p[y * w + x];
        assert!(at(128, 70) > 0.9, "the face: {}", at(128, 70));
        assert!(at(128, 130) > 0.9, "the shoulders: {}", at(128, 130));
        for (x, y) in [(10, 10), (5, 140), (250, 20), (240, 130)] {
            assert!(at(x, y) < 0.1, "the wall at {x},{y}: {}", at(x, y));
        }
        // onnxruntime gives 0.2382 for this picture (the converter's check).
        let mean = p.iter().sum::<f32>() / p.len() as f32;
        assert!((mean - 0.2382).abs() < 0.01, "mean {mean}");
    }

    #[test]
    fn a_frame_gives_a_mask_at_the_model_size() {
        let s = Segmenter::new().unwrap();
        let f = I420Frame::new(64, 36, vec![128; I420Frame::len_for(64, 36)]).unwrap();
        let m = s.mask(&f).unwrap();
        assert_eq!((m.width, m.height, m.data.len()), (256, 144, 256 * 144));
    }
}
