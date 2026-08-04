#![no_main]

use libfuzzer_sys::fuzz_target;
use runtime_resource::{
    Resource, ResourceHandle, ResourceKind, ResourceOwner, ResourceRights, ResourceTable,
};
use std::any::Any;

struct FuzzResource;

impl Resource for FuzzResource {
    fn close(&mut self) {}

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fuzz_target!(|input: &[u8]| {
    let owner = ResourceOwner::new(1).expect("fixed non-zero owner");
    let kind = ResourceKind::new("fuzz.resource").expect("fixed valid kind");
    let rights = ResourceRights::from_bits(1);
    let mut table = ResourceTable::new(owner, 32).expect("fixed bounded table");
    let mut handles: Vec<ResourceHandle> = Vec::new();
    for chunk in input.chunks(2).take(4_096) {
        let selector = chunk[0] % 3;
        let index = chunk.get(1).copied().unwrap_or_default() as usize;
        match selector {
            0 => {
                if let Ok(handle) = table.insert(kind.clone(), rights, FuzzResource) {
                    handles.push(handle);
                }
            }
            1 => {
                if let Some(handle) = handles.get(index % handles.len().max(1)).copied() {
                    let _ = table.with_resource(handle, &kind, rights, |_| ());
                }
            }
            _ => {
                if let Some(handle) = handles.get(index % handles.len().max(1)).copied() {
                    let _ = table.close(handle);
                }
            }
        }
    }
});
