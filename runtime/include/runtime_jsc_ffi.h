#ifndef ZINTL_RUNTIME_JSC_FFI_H
#define ZINTL_RUNTIME_JSC_FFI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct zjsc_engine zjsc_engine_t;
typedef void (*zjsc_notify_fn)(void *user_data);

enum {
  ZJSC_OK = 0,
  ZJSC_EMPTY = 1,
  ZJSC_BUFFER_TOO_SMALL = 2,
  ZJSC_INVALID_ARGUMENT = 3,
  ZJSC_INVALID_STATE = 4,
  ZJSC_QUOTA_EXCEEDED = 5,
  ZJSC_BACKEND = 255,
};

uint32_t zjsc_engine_new(
    uint32_t max_evaluations, uint32_t max_host_requests,
    uint32_t max_source_bytes, uint32_t max_event_bytes,
    zjsc_notify_fn notify, void *user_data, zjsc_engine_t **out_engine);
uint32_t zjsc_engine_start(zjsc_engine_t *engine);
uint32_t zjsc_engine_submit(
    zjsc_engine_t *engine, uint64_t evaluation_id,
    const uint8_t *source, size_t source_len);
uint32_t zjsc_engine_next_event(
    zjsc_engine_t *engine, uint8_t *output, size_t capacity,
    size_t *out_required);
uint32_t zjsc_engine_complete(
    zjsc_engine_t *engine, uint64_t request_id, uint32_t completion_kind,
    uint64_t object_id, const uint8_t *payload, size_t payload_len);
uint32_t zjsc_engine_cancel(zjsc_engine_t *engine, uint64_t evaluation_id);
uint32_t zjsc_engine_shutdown(zjsc_engine_t *engine);
void zjsc_engine_free(zjsc_engine_t *engine);

#ifdef __cplusplus
}
#endif

#endif
