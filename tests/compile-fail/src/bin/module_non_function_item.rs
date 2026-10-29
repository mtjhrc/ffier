//! expected-error: ffier module exports currently support only functions; other item kinds may be supported later

#[ffier::export]
pub mod logging {
    pub const MARKER: u32 = 42;

    pub fn init_log(target: u32) -> bool {
        target != 0
    }
}

fn main() {}
