#![no_main]

use libfuzzer_sys::fuzz_target;
use runtime_core::codec::{decode_completion, decode_request};
use runtime_ffi::{
    ABI_VERSION, RtConfig, RtRuntime, rt_runtime_free, rt_runtime_new, rt_runtime_shutdown,
    rt_runtime_start, rt_runtime_submit,
};
use std::ptr;

fuzz_target!(|input: &[u8]| {
    const MAX_PAYLOAD: usize = 64 * 1_024;
    let _ = decode_request(input, MAX_PAYLOAD);
    let _ = decode_completion(input, MAX_PAYLOAD);

    let config = RtConfig {
        abi_version: ABI_VERSION,
        flags: 0,
        max_inflight_requests: 2,
        max_completion_bytes: MAX_PAYLOAD as u32,
    };
    let mut runtime: *mut RtRuntime = ptr::null_mut();
    // SAFETY: Configuration/output and input slices are live for each call;
    // successful allocation is uniquely shut down and freed below.
    unsafe {
        if rt_runtime_new(&raw const config, &raw mut runtime).status == 0 {
            let _ = rt_runtime_start(runtime);
            let _ = rt_runtime_submit(runtime, 1, 1, input.as_ptr(), input.len());
            let _ = rt_runtime_shutdown(runtime);
            rt_runtime_free(runtime);
        }
    }
});
