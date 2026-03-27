// Swap between native Rust linking and C ABI dynamic linking:
//   cargo test -p ffier-test-consumer --features native
//   cargo test -p ffier-test-consumer --no-default-features --features via-cdylib

#[cfg(feature = "native")]
use ffier_test_lib as api;

#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

use api::{Config, Gadget, View, Widget};

fn make_widget() -> Widget {
    Widget::new()
}

fn make_named_widget(name: &str) -> Widget {
    Widget::with_name(name)
}

fn widget_roundtrip() -> i32 {
    let mut w = make_widget();
    w.set_count(42);
    w.get_count()
}

fn builder_chain() -> (String, i32) {
    let c = Config::new()
        .set_name("hello")
        .set_size(99);
    (c.get_name().to_owned(), c.get_size())
}

fn result_ok_path() -> Result<i32, api::TestError> {
    let w = make_widget();
    w.parse_count("test")
}

fn result_err_path() -> Result<(), api::TestError> {
    let w = make_widget();
    w.fail_always()
}

fn handle_param_roundtrip() -> i32 {
    let mut w = make_widget();
    w.set_count(7);
    let g: Gadget = w.create_gadget();
    w.read_gadget(&g)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constructor() {
        let w = make_widget();
        assert_eq!(w.get_count(), 0);
    }

    #[test]
    fn test_named_constructor() {
        let w = make_named_widget("hello");
        assert_eq!(w.name(), "hello");
    }

    #[test]
    fn test_roundtrip() {
        assert_eq!(widget_roundtrip(), 42);
    }

    #[test]
    fn test_builder() {
        let (name, size) = builder_chain();
        assert_eq!(name, "hello");
        assert_eq!(size, 99);
    }

    #[test]
    fn test_result_ok() {
        assert_eq!(result_ok_path().unwrap(), 4); // len("test")
    }

    #[test]
    fn test_result_err() {
        let err = result_err_path().unwrap_err();
        assert_eq!(err, api::TestError::CustomMessage);
    }

    #[test]
    fn test_handle_param() {
        assert_eq!(handle_param_roundtrip(), 7);
    }

    #[test]
    fn test_str_return() {
        let w = make_widget();
        assert_eq!(w.name(), "widget");
        assert_eq!(w.echo("ping"), "ping");
    }

    #[test]
    fn test_bytes_return() {
        let w = make_named_widget("abc");
        assert_eq!(w.data(), b"abc");
    }

    #[test]
    fn test_bool_return() {
        let w = make_widget();
        assert!(w.is_active());
    }

    #[test]
    fn test_str_slice_param() {
        let mut w = make_widget();
        w.set_tags(&["a", "b", "c"]);
        assert_eq!(w.tags_joined(), "a,b,c");
    }

    #[test]
    fn test_result_str() {
        let w = make_widget();
        assert_eq!(w.describe(0).unwrap(), "zero");
        assert!(w.describe(99).is_err());
    }

    #[test]
    fn test_mutable_handle_param() {
        let w = make_widget();
        let mut g = w.create_gadget();
        w.update_gadget(&mut g, 123);
        assert_eq!(g.get(), 123);
    }

    #[test]
    fn test_builder_validated_ok() {
        let c = Config::new()
            .set_name("valid")
            .validated()
            .unwrap();
        assert_eq!(c.get_name(), "valid");
    }

    #[test]
    fn test_builder_validated_err() {
        let result = Config::new().validated();
        assert!(result.is_err());
    }

    // ================================================================
    // Lifetime-parameterized types (View<'a> borrows Widget)
    // ================================================================

    #[test]
    fn test_view_borrows_widget() {
        let mut w = Widget::new();
        w.set_count(77);
        let v = View::create(&w);
        assert_eq!(v.source_count(), 77);
    }

    #[test]
    fn test_view_label() {
        let w = Widget::new();
        let mut v = View::create(&w);
        assert_eq!(v.label(), "default");
        v.set_label("custom");
        assert_eq!(v.label(), "custom");
    }

    #[test]
    fn test_view_lifetime_enforced() {
        // This should compile: view lives shorter than widget
        let mut w = Widget::new();
        w.set_count(42);
        let count = {
            let v = View::create(&w);
            v.source_count()
        };
        assert_eq!(count, 42);
    }
}
