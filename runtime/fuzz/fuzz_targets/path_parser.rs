#![no_main]

use libfuzzer_sys::fuzz_target;
use runtime_filesystem::RelativePath;

fuzz_target!(|input: &[u8]| {
    if input.len() <= 8 * 1_024 {
        if let Ok(text) = std::str::from_utf8(input) {
            let _ = RelativePath::parse(text);
        }
    }
});
