/// Rectangle in logical points / DIP.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct PhysicalSize {
    pub width: u32,
    pub height: u32,
}
