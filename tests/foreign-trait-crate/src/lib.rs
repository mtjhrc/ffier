/// A simple trait defined in an external crate, with no ffier annotations.
pub trait Weighable {
    fn weight_grams(&self) -> i32;
}

#[ffier::export]
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEventKind {
    Key = 1,
    Absolute = 3,
}

ffier::export_bitflags! {
    bitflags::bitflags! {
        #[repr(transparent)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct InputEventFlags: u32 {
            const REPEAT = 0b01;
            const SYNTHETIC = 0b10;
        }
    }
}

#[ffier::export]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputDeviceIds {
    pub bus: u16,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
}

#[ffier::export]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputAbsInfo {
    pub value: i32,
    pub minimum: i32,
    pub maximum: i32,
    pub fuzz: i32,
    pub flat: i32,
    pub resolution: i32,
}

#[ffier::export]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputEvent {
    pub device: InputDeviceIds,
    pub kind: InputEventKind,
    pub code: u16,
    pub flags: InputEventFlags,
    pub value: i32,
}

#[ffier::export]
pub trait InputSource {
    #[ffier(index = 0)]
    fn poll_event(&mut self) -> Option<InputEvent>;

    #[ffier(index = 1)]
    fn device_ids(&self) -> InputDeviceIds;

    #[ffier(index = 2)]
    fn abs_info(&self) -> &InputAbsInfo;

    #[ffier(index = 3)]
    fn abs_info_mut(&mut self) -> &mut InputAbsInfo;

    #[ffier(index = 4)]
    fn optional_abs_info(&self, available: bool) -> Option<&InputAbsInfo>;

    #[ffier(index = 5)]
    fn submit_event(&mut self, event: InputEvent);
}
