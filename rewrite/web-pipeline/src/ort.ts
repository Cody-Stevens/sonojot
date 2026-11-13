// The small slice of ONNX Runtime this package uses. onnxruntime-web (browser)
// and onnxruntime-node (tests) both satisfy it, so engine code never imports a
// specific runtime.
import type { InferenceSession, Tensor } from "onnxruntime-common";

export type { InferenceSession, Tensor };

export interface Ort {
  Tensor: typeof Tensor;
}

export type Session = Pick<InferenceSession, "run" | "release" | "inputNames" | "outputNames">;
