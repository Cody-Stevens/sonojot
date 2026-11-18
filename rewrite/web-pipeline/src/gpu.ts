/** The parts of a WebGPU adapter the device choice reads. */
export interface AdapterLike {
  /** Older browsers put the flag on the adapter itself. */
  isFallbackAdapter?: boolean;
  /** The current spec puts it on `adapter.info`. */
  info?: { isFallbackAdapter?: boolean };
}

/**
 * Whether `device: "auto"` should run on this adapter. A fallback adapter is a
 * software rasterizer (Chrome's SwiftShader, for example, when the GPU is
 * blocklisted or Vulkan is off), which runs the speech model far slower than
 * WebAssembly does.
 */
export function isHardwareAdapter(adapter: AdapterLike | null | undefined): boolean {
  if (!adapter) return false;
  return !(adapter.info?.isFallbackAdapter ?? adapter.isFallbackAdapter ?? false);
}
