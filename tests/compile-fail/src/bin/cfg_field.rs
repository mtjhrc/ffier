#[ffier::export]
#[repr(C)]
pub struct CfgField {
    pub always: u32,
    #[cfg(any())]
    pub conditional: u32,
}

fn main() {}
