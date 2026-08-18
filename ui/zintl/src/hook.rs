#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HookId(u32);

impl HookId {
    pub const DEFAULT: Self = HookId(0);

    pub fn new(id: u32) -> Self {
        HookId(id)
    }

    pub fn value(&self) -> u32 {
        self.0
    }
}

pub trait Hook {
    fn get_id(&self) -> HookId;
}
