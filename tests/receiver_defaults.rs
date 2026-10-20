// Compile unchanged against native, strong, and weak clients.
use std::cell::Cell;
use std::rc::Rc;

use api::ReceiverDefaults as _;

struct BorrowedDefaults<'a> {
    label: &'a str,
    adjustments: Rc<Cell<usize>>,
}

impl api::ReceiverDefaults for BorrowedDefaults<'_> {
    fn label(&self) -> &str {
        self.label
    }

    fn adjust(&mut self) {
        self.adjustments.set(self.adjustments.get() + 1);
    }
}

struct OverriddenDefaults<'a>(BorrowedDefaults<'a>);

impl api::ReceiverDefaults for OverriddenDefaults<'_> {
    fn label(&self) -> &str {
        self.0.label()
    }

    fn adjust(&mut self) {
        self.0.adjust();
    }

    fn choose<'a>(&'a self, _fallback: &'a str) -> &'a str {
        self.label()
    }

    fn choose_mut<'a>(&'a mut self, _fallback: &'a str) -> &'a str {
        self.adjust();
        self.label()
    }
}

#[test]
fn defaults_preserve_shared_and_mutable_receiver_borrows() {
    init();
    let mut widget = api::Widget::with_name("receiver");
    let fallback = String::from("fallback value");
    assert_eq!(widget.choose(""), "receiver");
    assert_eq!(widget.choose(&fallback), fallback);
    assert_eq!(widget.choose_mut(""), "receiver");
    assert_eq!(widget.get_count(), 1);
    assert_eq!(widget.choose_mut(&fallback), fallback);
    assert_eq!(widget.get_count(), 2);
    assert_eq!(widget.choose_elided(&fallback), "receiver");
    assert_eq!(widget.choose_mut_elided(&fallback), "receiver");
    assert_eq!(widget.get_count(), 3);
}

#[test]
fn borrowed_callbacks_use_defaults_and_overrides() {
    init();
    let label = String::from("borrowed receiver");
    let adjustments = Rc::new(Cell::new(0));
    let callback = || BorrowedDefaults {
        label: &label,
        adjustments: adjustments.clone(),
    };
    assert_eq!(api::probe_receiver_defaults(callback(), ""), 2 * label.len());
    assert_eq!(adjustments.get(), 1);
    let fallback = String::from("alternative");
    assert_eq!(api::probe_receiver_defaults(callback(), &fallback), 2 * fallback.len());
    assert_eq!(adjustments.get(), 2);
    assert_eq!(
        api::probe_receiver_defaults(OverriddenDefaults(callback()), &fallback),
        2 * label.len(),
    );
    assert_eq!(adjustments.get(), 3);
}
