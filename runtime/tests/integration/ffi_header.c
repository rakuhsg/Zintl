#include "../../include/runtime_ffi.h"

_Static_assert(sizeof(rt_status_t) == 8, "rt_status_t ABI changed");
_Static_assert(sizeof(rt_config_t) == 16, "rt_config_t ABI changed");
_Static_assert(RT_ABI_VERSION == 1u, "unexpected ABI version");

static void notify(void *user_data) { (void)user_data; }

int ffi_header_smoke(void) {
  rt_runtime_t *runtime = 0;
  rt_config_t config = {RT_ABI_VERSION, 0, 1, 1024};
  rt_status_t status = rt_runtime_new(&config, &runtime);
  if (status.status != RT_OK) {
    return 1;
  }
  (void)rt_runtime_set_notifier(runtime, notify, 0);
  (void)rt_runtime_start(runtime);
  (void)rt_runtime_submit_timer(runtime, 1, 2, 10);
  uint32_t present = 0;
  uint64_t deadline = 0;
  uint32_t fired = 0;
  (void)rt_runtime_next_timer_deadline(runtime, &present, &deadline);
  (void)rt_runtime_fire_due_timers(runtime, deadline, 1, &fired);
  (void)rt_runtime_shutdown(runtime);
  rt_runtime_free(runtime);
  return 0;
}
