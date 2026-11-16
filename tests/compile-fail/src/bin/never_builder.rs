//! expected-error: #[ffier(never_type = ...)] cannot be combined with a returned Self or a foreign return

pub struct Builder;

#[ffier::export]
impl Builder {
    #[ffier(never_type = Self)]
    pub fn update(self) -> Self {
        self
    }
}

fn main() {}
