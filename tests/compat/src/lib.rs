// Vtable backward-compatibility test.
//
// This crate includes "v1" generated client bindings where the Fruit trait
// does NOT have a `label` method (and FruitVtable has no `label` field).
// It links against the current (v2) cdylib which DOES have `label` with a
// default implementation returning "fruit".
//
// The test verifies that when the library calls `label()` on a handle created
// by this v1 client, it sees the vtable field as out of bounds (smaller
// vtable_size), treats it as None, and uses the library's default.
#![allow(clippy::all)]

include!("generated_v1.rs");

#[cfg(test)]
mod tests {
    use super::*;

    struct Pear {
        weight: i32,
    }

    impl Fruit for Pear {
        fn value(&self) -> i32 {
            self.weight
        }
    }

    #[test]
    fn v1_custom_type_gets_v2_default_for_label() {
        // Pear implements v1 Fruit (only `value`). When passed to
        // fruit_label_len, the library calls label() on the handle.
        // The v1 vtable is smaller — no label field — so the library
        // uses its default implementation ("fruit", len 5).
        let m = Mixer::new();
        assert_eq!(m.fruit_label_len(Pear { weight: 42 }), 5);
    }

    #[test]
    fn v1_custom_type_value_still_works() {
        // Verify that value() still dispatches correctly through the
        // v1 vtable even though the library has a larger vtable struct.
        let m = Mixer::new().add(Pear { weight: 17 });
        assert_eq!(m.total(), 17);
    }
}
