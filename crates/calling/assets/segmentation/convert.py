#!/usr/bin/env python3
"""Converts MediaPipe's selfie segmentation (landscape) model from TFLite to ONNX.

The source is Google's `selfie_segmentation_landscape.tflite` (Apache-2.0):
https://storage.googleapis.com/mediapipe-assets/selfie_segmentation_landscape.tflite

    pip install tflite onnx numpy
    python3 convert.py selfie_segmentation_landscape.tflite selfie_segmentation.onnx

The ONNX graph takes NCHW input (1x3x144x256 RGB in 0..1) and returns the person probability as
1x1x144x256. Weights stored as float16 in the TFLite file become float32. Opset 13, with only the
operators tract-onnx runs (HardSwish is written as HardSigmoid times x). MediaPipe's custom
`Convolution2DTransposeBias` is an ONNX ConvTranspose.
"""

import sys

import numpy as np
import onnx
import tflite
from onnx import TensorProto, helper, numpy_helper

OPS = {v: k for k, v in tflite.BuiltinOperator.__dict__.items() if not k.startswith("_")}
# NHWC axis -> NCHW axis.
AXIS = {0: 0, 1: 2, 2: 3, 3: 1}


def main(src, dst):
    buf = open(src, "rb").read()
    m = tflite.Model.GetRootAsModel(buf, 0)
    g = m.Subgraphs(0)
    consts = {}
    nodes = []
    inits = []

    def tensor(i):
        return g.Tensors(i)

    def shape(i):
        return tensor(i).ShapeAsNumpy().tolist()

    def data(i):
        t = tensor(i)
        raw = m.Buffers(t.Buffer()).DataAsNumpy()
        if isinstance(raw, int) or raw.size == 0:
            return None
        dtype = {0: np.float32, 1: np.float16, 2: np.int32, 4: np.int64}[t.Type()]
        return np.frombuffer(raw.tobytes(), dtype=dtype).reshape(shape(i))

    def const(i):
        if i in consts:
            return consts[i]
        d = data(i)
        if d is None:
            raise ValueError(f"tensor {i} is not a constant")
        return d

    def name(i):
        return f"t{i}"

    def init(n, arr):
        inits.append(numpy_helper.from_array(arr, n))
        return n

    def options(op, cls):
        o = cls()
        t = op.BuiltinOptions()
        o.Init(t.Bytes, t.Pos)
        return o

    def same_pads(i_in, i_out, k, s):
        ih, iw = shape(i_in)[1:3]
        oh, ow = shape(i_out)[1:3]
        pads = []
        for size, out, kk, ss in ((ih, oh, k[0], s[0]), (iw, ow, k[1], s[1])):
            total = max((out - 1) * ss + kk - size, 0)
            pads.append((total // 2, total - total // 2))
        return [pads[0][0], pads[1][0], pads[0][1], pads[1][1]]

    def activation(out, act):
        if act == 0:
            return out
        if act != 1:
            raise ValueError(f"fused activation {act}")
        nodes.append(helper.make_node("Relu", [out + "_pre"], [out]))
        return out + "_pre"

    for n in range(g.OperatorsLength()):
        op = g.Operators(n)
        oc = m.OperatorCodes(op.OpcodeIndex())
        code = max(oc.BuiltinCode(), oc.DeprecatedBuiltinCode())
        ins = [op.Inputs(j) for j in range(op.InputsLength())]
        out = op.Outputs(0)
        kind = OPS.get(code)
        if kind == "DEQUANTIZE":
            consts[out] = data(ins[0]).astype(np.float32)
            continue
        o = name(out)
        if kind in ("CONV_2D", "DEPTHWISE_CONV_2D"):
            depthwise = kind == "DEPTHWISE_CONV_2D"
            opt = options(op, tflite.DepthwiseConv2DOptions if depthwise else tflite.Conv2DOptions)
            w = const(ins[1])
            b = const(ins[2])
            if depthwise:
                if w.shape[3] != shape(ins[0])[3]:
                    raise ValueError("depth multiplier other than 1")
                wt = w[0].transpose(2, 0, 1)[:, None]
            else:
                wt = w.transpose(0, 3, 1, 2)
            k = list(wt.shape[2:])
            s = [opt.StrideH(), opt.StrideW()]
            pads = same_pads(ins[0], out, k, s) if opt.Padding() == 0 else [0, 0, 0, 0]
            target = activation(o, opt.FusedActivationFunction())
            nodes.append(
                helper.make_node(
                    "Conv",
                    [name(ins[0]), init(o + "_w", wt.astype(np.float32)), init(o + "_b", b.astype(np.float32))],
                    [target],
                    kernel_shape=k,
                    strides=s,
                    pads=pads,
                    dilations=[opt.DilationHFactor(), opt.DilationWFactor()],
                    group=wt.shape[0] if depthwise else 1,
                )
            )
        elif kind == "RELU":
            nodes.append(helper.make_node("Relu", [name(ins[0])], [o]))
        elif kind == "LOGISTIC":
            nodes.append(helper.make_node("Sigmoid", [name(ins[0])], [o]))
        elif kind == "HARD_SWISH":
            nodes.append(helper.make_node("HardSigmoid", [name(ins[0])], [o + "_gate"], alpha=1.0 / 6, beta=0.5))
            nodes.append(helper.make_node("Mul", [name(ins[0]), o + "_gate"], [o]))
        elif kind in ("ADD", "MUL"):
            opt = options(op, tflite.AddOptions if kind == "ADD" else tflite.MulOptions)
            target = activation(o, opt.FusedActivationFunction())
            nodes.append(helper.make_node(kind.title(), [name(ins[0]), name(ins[1])], [target]))
        elif kind == "MEAN":
            opt = options(op, tflite.ReducerOptions)
            axes = sorted(AXIS[int(a)] for a in const(ins[1]).ravel())
            nodes.append(
                helper.make_node("ReduceMean", [name(ins[0])], [o], axes=axes, keepdims=int(opt.KeepDims()))
            )
        elif kind == "RESIZE_BILINEAR":
            opt = options(op, tflite.ResizeBilinearOptions)
            if opt.AlignCorners():
                mode = "align_corners"
            elif opt.HalfPixelCenters():
                mode = "half_pixel"
            else:
                mode = "asymmetric"
            n_, h, w_, c = shape(out)
            sizes = init(o + "_sizes", np.array([n_, c, h, w_], dtype=np.int64))
            nodes.append(
                helper.make_node(
                    "Resize",
                    [name(ins[0]), "", "", sizes],
                    [o],
                    mode="linear",
                    coordinate_transformation_mode=mode,
                )
            )
        elif code == tflite.BuiltinOperator.CUSTOM and oc.CustomCode() == b"Convolution2DTransposeBias":
            # MediaPipe's TfLiteTransposeConvParams: padding (1 SAME, 2 VALID), stride w, stride h.
            padding, stride_w, stride_h = np.frombuffer(op.CustomOptionsAsNumpy().tobytes(), dtype=np.int32)
            w = const(ins[1])
            b = const(ins[2])
            wt = w.transpose(3, 0, 1, 2)
            k = list(wt.shape[2:])
            ih, iw = shape(ins[0])[1:3]
            oh, ow = shape(out)[1:3]
            pads = [0, 0, 0, 0]
            if padding == 1:
                th = (ih - 1) * stride_h + k[0] - oh
                tw = (iw - 1) * stride_w + k[1] - ow
                pads = [th // 2, tw // 2, th - th // 2, tw - tw // 2]
            nodes.append(
                helper.make_node(
                    "ConvTranspose",
                    [name(ins[0]), init(o + "_w", wt.astype(np.float32)), init(o + "_b", b.astype(np.float32))],
                    [o],
                    kernel_shape=k,
                    strides=[int(stride_h), int(stride_w)],
                    pads=[int(p) for p in pads],
                )
            )
        else:
            raise ValueError(f"unsupported operator {kind or oc.CustomCode()}")

    src_in = g.Inputs(0)
    src_out = g.Outputs(0)
    _, h, w, c = shape(src_in)
    # The graph's input feeds the first nodes under its TFLite name.
    nodes.insert(0, helper.make_node("Identity", ["input"], [name(src_in)]))
    nodes.append(helper.make_node("Identity", [name(src_out)], ["mask"]))
    graph = helper.make_graph(
        nodes,
        "selfie_segmentation_landscape",
        [helper.make_tensor_value_info("input", TensorProto.FLOAT, [1, c, h, w])],
        [helper.make_tensor_value_info("mask", TensorProto.FLOAT, [1, 1, h, w])],
        inits,
    )
    model = helper.make_model(
        graph,
        opset_imports=[helper.make_opsetid("", 13)],
        producer_name="ironteams convert.py",
        doc_string="MediaPipe selfie segmentation (landscape), Apache-2.0, converted from TFLite.",
    )
    model.ir_version = 8
    onnx.checker.check_model(model)
    onnx.save(model, dst)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
