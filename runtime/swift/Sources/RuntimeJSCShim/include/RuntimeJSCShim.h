#ifndef ZINTL_RUNTIME_JSC_SHIM_H
#define ZINTL_RUNTIME_JSC_SHIM_H

#include <JavaScriptCore/JavaScriptCore.h>
#include <stdint.h>

JSObjectRef rtjsc_host_object_make(
    JSContextRef context, uint64_t runtime_id, uint64_t object_id,
    uint32_t kind);

uint32_t rtjsc_host_object_read(
    JSContextRef context, JSValueRef value, uint64_t expected_runtime_id,
    uint32_t expected_kind, uint64_t *out_object_id);

#endif
