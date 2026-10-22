//! expected-error: cannot be shared between threads safely

fn require_sync<T: Sync>() {}

fn main() {
    require_sync::<ffier::VtableHandle>();
}
