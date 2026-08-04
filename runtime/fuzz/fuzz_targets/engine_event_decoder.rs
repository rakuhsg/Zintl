#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _ = runtime_engine::wire::decode_event(input);
});
