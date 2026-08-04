#include "RuntimeJSCShim.h"

#include <dispatch/dispatch.h>
#include <stdlib.h>

typedef struct {
  uint64_t runtime_id;
  uint64_t object_id;
  uint32_t kind;
} rtjsc_host_private_t;

static void rtjsc_finalize(JSObjectRef object) {
  free(JSObjectGetPrivate(object));
}

static JSClassRef rtjsc_host_class(void) {
  static JSClassRef host_class;
  static dispatch_once_t once;
  dispatch_once(&once, ^{
    JSClassDefinition definition = kJSClassDefinitionEmpty;
    definition.className = "ZintlOpaqueHostObject";
    definition.finalize = rtjsc_finalize;
    host_class = JSClassCreate(&definition);
  });
  return host_class;
}

JSObjectRef rtjsc_host_object_make(
    JSContextRef context, uint64_t runtime_id, uint64_t object_id,
    uint32_t kind) {
  if (context == NULL || runtime_id == 0 || object_id == 0 || kind == 0) {
    return NULL;
  }
  rtjsc_host_private_t *private_data = calloc(1, sizeof(*private_data));
  if (private_data == NULL) {
    return NULL;
  }
  private_data->runtime_id = runtime_id;
  private_data->object_id = object_id;
  private_data->kind = kind;
  JSObjectRef object = JSObjectMake(context, rtjsc_host_class(), private_data);
  if (object == NULL) {
    free(private_data);
  }
  return object;
}

uint32_t rtjsc_host_object_read(
    JSContextRef context, JSValueRef value, uint64_t expected_runtime_id,
    uint32_t expected_kind, uint64_t *out_object_id) {
  if (context == NULL || value == NULL || out_object_id == NULL ||
      expected_runtime_id == 0 || expected_kind == 0 ||
      !JSValueIsObject(context, value)) {
    return 1;
  }
  JSValueRef exception = NULL;
  JSObjectRef object = JSValueToObject(context, value, &exception);
  if (object == NULL || exception != NULL) {
    return 1;
  }
  rtjsc_host_private_t *private_data = JSObjectGetPrivate(object);
  if (private_data == NULL) {
    return 2;
  }
  if (private_data->runtime_id != expected_runtime_id) {
    return 3;
  }
  if (private_data->kind != expected_kind) {
    return 4;
  }
  *out_object_id = private_data->object_id;
  return 0;
}
