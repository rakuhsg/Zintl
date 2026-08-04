#ifndef ZINTL_RUNTIME_FFI_H
#define ZINTL_RUNTIME_FFI_H

#include <stddef.h>
#include <stdint.h>

#define RT_ABI_VERSION 1u
#define RT_OK 0u
#define RT_EMPTY 1u
#define RT_BUFFER_TOO_SMALL 2u
#define RT_INVALID_ARGUMENT 3u
#define RT_INVALID_STATE 4u
#define RT_DUPLICATE_REQUEST 5u
#define RT_UNKNOWN_REQUEST 6u
#define RT_QUOTA_EXCEEDED 7u
#define RT_RUNTIME_SHUTTING_DOWN 8u
#define RT_OPERATION_FAILED 9u
#define RT_INTERNAL 255u

typedef struct rt_runtime rt_runtime_t;

typedef struct {
  uint32_t abi_version;
  uint32_t flags;
  uint32_t max_inflight_requests;
  uint32_t max_completion_bytes;
} rt_config_t;

typedef struct {
  uint32_t status;
  uint32_t detail;
} rt_status_t;

typedef void (*rt_notifier_fn)(void *user_data);

rt_status_t rt_runtime_new(const rt_config_t *, rt_runtime_t **out_runtime);
rt_status_t rt_runtime_set_notifier(
    rt_runtime_t *, rt_notifier_fn callback, void *user_data);
rt_status_t rt_runtime_start(rt_runtime_t *);
rt_status_t rt_runtime_submit(
    rt_runtime_t *, uint64_t request_id, uint32_t op_id,
    const uint8_t *payload, size_t payload_len);
rt_status_t rt_runtime_submit_timer(
    rt_runtime_t *, uint64_t request_id, uint32_t op_id,
    uint64_t deadline_tick);
rt_status_t rt_runtime_fire_due_timers(
    rt_runtime_t *, uint64_t now_tick, uint32_t maximum,
    uint32_t *out_fired);
rt_status_t rt_runtime_next_timer_deadline(
    rt_runtime_t *, uint32_t *out_present, uint64_t *out_deadline_tick);
rt_status_t rt_runtime_fs_open_approved_directory(
    rt_runtime_t *, const uint8_t *locator, size_t locator_len,
    uint64_t rights, uint64_t *out_object_id);
rt_status_t rt_runtime_fs_directory_identity(
    rt_runtime_t *, uint64_t object_id,
    uint8_t *out_identity, size_t capacity);
rt_status_t rt_runtime_fs_open_imported_directory(
    rt_runtime_t *, const uint8_t *locator, size_t locator_len,
    uint64_t rights, const uint8_t *identity, size_t identity_len,
    uint64_t *out_object_id);
rt_status_t rt_runtime_fs_open_relative(
    rt_runtime_t *, uint64_t directory_object_id,
    const uint8_t *path, size_t path_len, uint64_t rights,
    uint32_t create, uint32_t truncate, uint64_t *out_file_object_id);
rt_status_t rt_runtime_fs_read(
    rt_runtime_t *, uint64_t request_id, uint64_t file_object_id,
    uint32_t max_bytes);
rt_status_t rt_runtime_fs_write(
    rt_runtime_t *, uint64_t request_id, uint64_t file_object_id,
    const uint8_t *data, size_t data_len);
rt_status_t rt_runtime_fs_stat(
    rt_runtime_t *, uint64_t request_id, uint64_t file_object_id);
rt_status_t rt_runtime_fs_read_file(
    rt_runtime_t *, uint64_t request_id, uint64_t object_id,
    const uint8_t *path, size_t path_len, uint32_t max_bytes);
rt_status_t rt_runtime_fs_write_file(
    rt_runtime_t *, uint64_t request_id, uint64_t object_id,
    const uint8_t *path, size_t path_len,
    const uint8_t *data, size_t data_len,
    uint32_t create, uint32_t truncate);
rt_status_t rt_runtime_fs_metadata(
    rt_runtime_t *, uint64_t request_id, uint64_t object_id,
    const uint8_t *path, size_t path_len);
rt_status_t rt_runtime_fs_close(
    rt_runtime_t *, uint64_t request_id, uint64_t object_id);
rt_status_t rt_runtime_pump_filesystem(
    rt_runtime_t *, uint32_t maximum, uint32_t *out_pumped);
rt_status_t rt_runtime_next_completion(
    rt_runtime_t *, uint8_t *out, size_t capacity,
    size_t *required_or_written);
rt_status_t rt_runtime_complete_host_op(
    rt_runtime_t *, uint64_t request_id, uint32_t status,
    const uint8_t *payload, size_t payload_len);
rt_status_t rt_runtime_cancel(rt_runtime_t *, uint64_t request_id);
rt_status_t rt_runtime_shutdown(rt_runtime_t *);
void rt_runtime_free(rt_runtime_t *);

/* All functions except free return stable status values and never unwind. */

#endif
