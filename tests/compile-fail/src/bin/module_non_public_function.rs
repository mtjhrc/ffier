//! expected-error: ffier module functions must be `pub`

#[ffier::export]
pub mod logging {
    fn helper(target: u32) -> u32 {
        target
    }

    pub fn init_log(target: u32) -> bool {
        helper(target) != 0
    }
}

fn main() {}
