# Person segmentation model

`selfie_segmentation.onnx` is MediaPipe's selfie segmentation model, landscape variant (input
256x144), converted from TensorFlow Lite to ONNX for tract. IronTeams uses it on the CPU for the
background blur and background replacement of the camera (`src/video/effects`).

- Source: Google, `selfie_segmentation_landscape.tflite`,
  https://storage.googleapis.com/mediapipe-assets/selfie_segmentation_landscape.tflite
  (sha256 `a77d03f4659b9f6b6c1f5106947bf40e99d7655094b6527f214ea7d451106edd`)
- Licence: Apache License 2.0 (the MediaPipe repository and the model card; full text in
  `LICENSE` next to this file). The converted file is a derivative work under the same licence.
- Changes: converted by `convert.py` in this folder (float16 weights stored as float32; NCHW input
  1x3x144x256 RGB in 0..1, output 1x1x144x256 person probability; HardSwish written as
  HardSigmoid times x; MediaPipe's custom `Convolution2DTransposeBias` as an ONNX ConvTranspose).
  No weights were changed.
- Check: on the same input, its output matches MediaPipe's own `ImageSegmenter` run of the TFLite
  file to within 2e-4 (max absolute difference 1.7e-4, onnxruntime 1.32).
- sha256 of `selfie_segmentation.onnx`:
  `a40cdb12b7c98a26082f987263e2658c7adc20b16ef4f819846aa7822ff6bc5a`

The model card describes the intended use (one or two people close to a webcam, as in video
calls) and its limits (accuracy drops for people far from the camera and in some poses).
