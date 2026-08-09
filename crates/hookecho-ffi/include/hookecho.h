/* Hook Echo C ABI. Link against libhookecho_ffi (cdylib or staticlib).
   ponytail: cbindgen when the surface grows. */
#ifndef HOOKECHO_H
#define HOOKECHO_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Decode a NEXRAD Level 3 product to JSON. Returns a NUL-terminated string that the caller must
   release with hookecho_string_free; on a decode failure the JSON is {"error": "..."}, and the
   result is NULL only if allocation failed. */
char *hookecho_l3_decode_json(const uint8_t *data, size_t len);

/* Release a string returned by this library. NULL is a no-op. */
void hookecho_string_free(char *s);

#ifdef __cplusplus
}
#endif

#endif /* HOOKECHO_H */
