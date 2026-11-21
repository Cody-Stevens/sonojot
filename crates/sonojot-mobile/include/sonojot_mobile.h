/*
 * sonojot-mobile C ABI, version 2 (sonojot_mobile_abi_version()).
 *
 * Strings are UTF-8 and NUL-terminated. Every char* a function returns belongs to the caller
 * and must be released with sonojot_string_free(). On failure a function returns NULL and, when
 * out_error is not NULL, stores a JSON object there: {"code": "...", "message": "..."}.
 *
 * Error codes: runtime_missing, runtime_symbol, runtime_version, pack_missing, pack_invalid,
 * pack_verification, unsupported_locale, init, decode, invalid_audio, invalid_argument, panic.
 *
 * Audio is 16 kHz mono, at most 30 minutes per call (longer audio fails with invalid_audio).
 * An engine handle may be used from any thread; calls on one handle are serialized. Close it
 * with sonojot_engine_close() once no other call on it is running or can still start.
 *
 * sonojot_engine_open, sonojot_pack_inspect (full != 0) and
 * sonojot_pack_finalize hash hundreds of megabytes, and transcription takes seconds: call them
 * from a background thread, never the UI thread.
 */
#ifndef SONOJOT_MOBILE_H
#define SONOJOT_MOBILE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct SonojotEngineHandle SonojotEngineHandle;

uint32_t sonojot_mobile_abi_version(void);

/* Static string; do not free. */
const char *sonojot_mobile_version(void);

/* Static JSON string; do not free. What this library build expects, so apps need not copy it:
 *   {"abi_version": 2, "manifest_schema": 1, "required_sherpa_version": "1.13.8"}
 * abi_version equals sonojot_mobile_abi_version(); manifest_schema is the manifest.json
 * "schema" it accepts; required_sherpa_version is the exact SherpaOnnxGetVersionStr() of the
 * libsherpa-onnx-c-api.so it loads. */
const char *sonojot_mobile_abi_info(void);

/*
 * config_json fields (all optional except pack_dir and, for multilingual packs, locale):
 *   pack_dir         directory holding manifest.json
 *   locale           "en-US", "ar-JO", "tr-TR"...
 *   runtime_library  sherpa-onnx C API file name or path (default libsherpa-onnx-c-api.so)
 *   num_threads      1-8 (default 2)
 *   verify           "full" (default and only accepted value: every file's SHA-256 is checked
 *                    before native code reads it; "sizes" fails with invalid_argument)
 *   gate             {"mode": "rnnoise"|"off", "max_segment_ms": 28000, ...}
 *   written_form     true (default): English numbers, dates and money in written form
 */
SonojotEngineHandle *sonojot_engine_open(const char *config_json, char **out_error);
/* runtime_library may also be "self": the sherpa-onnx C API is then resolved from the running
 * program, for a runtime linked statically (iOS). Keep its SherpaOnnx* symbols exported.
 *
 * The locale's language picks the decoder language. Whisper gets a language only when it has a
 * token for it (nb becomes no, fil becomes tl, jv becomes jw; Android's iw, in and ji are read as
 * he, id and yi); otherwise it detects the language itself. It never translates. */

/* pcm: little-endian float32 bytes, any alignment; NULL is allowed when len is 0.
 * Returns a TranscriptResult JSON object:
 * {"text", "raw_text", "language", "model_id", "segments": [{"start_ms", "end_ms", "text"}],
 *  "audio_ms", "speech_ms", "gate_mode", "gate_fail_open"} */
char *sonojot_engine_transcribe_f32le(SonojotEngineHandle *engine, const uint8_t *pcm, size_t len, char **out_error);

/* samples: native float32, 4-byte aligned (a misaligned pointer fails with invalid_argument);
 * NULL is allowed when count is 0. Same result as sonojot_engine_transcribe_f32le. */
char *sonojot_engine_transcribe_f32(SonojotEngineHandle *engine, const float *samples, size_t count, char **out_error);

/* {"language": "...", "manifest": {...}} */
char *sonojot_engine_info(SonojotEngineHandle *engine, char **out_error);

/* NULL is ignored. Must not race with another call on the same handle: the caller makes sure
 * every call on it has returned and none can start, then closes it once. */
void sonojot_engine_close(SonojotEngineHandle *engine);

/* Validates pack_dir/manifest.json and checks file sizes (full == 0) or sizes and SHA-256.
 * Returns {"manifest": {...}, "report": {"pack_id", "files", "bytes", "hashed"}}. */
char *sonojot_pack_inspect(const char *pack_dir, int32_t full, char **out_error);

/* The download catalog, never NULL:
 * {"download_hosts": [...], "max_redirects": 8, "packs": [{"id", "display_name", "languages",
 *  "family", "bytes", "license", "files": [{"role", "path", "url", "bytes", "sha256"}]}]}
 *
 * Download rules: start only at a file's "url"; HTTPS only, including every redirect; follow at
 * most max_redirects redirects; every URL's host must match download_hosts, where an entry
 * starting with "." matches any subdomain of the rest ("cdn-lfs.hf.co" matches ".hf.co") and
 * any other entry matches exactly. Hash while downloading, then call sonojot_pack_finalize. */
char *sonojot_catalog(void);

/* After downloading catalog pack pack_id into staging_dir: verifies every file (size and
 * SHA-256), then writes manifest.json (through a temporary file renamed into place, so a link
 * planted there is replaced, never followed). Returns {"pack_id", "files", "bytes", "hashed"};
 * NULL and nothing written if any file is wrong. staging_dir must be in the app's private
 * internal storage, never shared or external storage. Move the directory into place only after
 * success. */
char *sonojot_pack_finalize(const char *staging_dir, const char *pack_id, char **out_error);

void sonojot_string_free(char *text);

#ifdef __cplusplus
}
#endif

#endif /* SONOJOT_MOBILE_H */
