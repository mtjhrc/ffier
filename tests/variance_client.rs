use api::{ContravariantConsumer, CovariantText, MixedVariance};

#[allow(clippy::needless_lifetimes)]
fn shorten<'long: 'short, 'short>(
    value: CovariantText<'long>,
    _scope: &'short (),
) -> CovariantText<'short> {
    value
}

#[allow(clippy::needless_lifetimes)]
fn shorten_shared<'long: 'short, 'short, 'borrow>(
    value: &'borrow CovariantText<'long>,
) -> &'borrow CovariantText<'short> {
    value
}

#[allow(clippy::needless_lifetimes)]
fn lengthen<'long: 'short, 'short>(
    value: ContravariantConsumer<'short>,
) -> ContravariantConsumer<'long> {
    value
}

#[allow(clippy::needless_lifetimes)]
fn lengthen_shared<'long: 'short, 'short, 'borrow>(
    value: &'borrow ContravariantConsumer<'short>,
) -> &'borrow ContravariantConsumer<'long> {
    value
}

// Each coercion changes exactly one parameter and holds every other one fixed.
#[allow(clippy::needless_lifetimes)]
fn mixed_covariant<'long: 'short, 'short, 'b, 'c, 'd>(
    value: MixedVariance<'long, 'b, 'c, 'd>,
) -> MixedVariance<'short, 'b, 'c, 'd> {
    value
}

#[allow(clippy::needless_lifetimes)]
fn mixed_contravariant<'long: 'short, 'short, 'a, 'c, 'd>(
    value: MixedVariance<'a, 'short, 'c, 'd>,
) -> MixedVariance<'a, 'long, 'c, 'd> {
    value
}

#[allow(clippy::needless_lifetimes)]
fn mixed_second_covariant<'long: 'short, 'short, 'a, 'b, 'c>(
    value: MixedVariance<'a, 'b, 'c, 'long>,
) -> MixedVariance<'a, 'b, 'c, 'short> {
    value
}

#[test]
fn covariance_preserves_borrowed_data() {
    init();
    let source = String::from("borrowed text");
    let value = CovariantText::new(&source);
    let scope = ();
    let value = shorten(value, &scope);
    assert_eq!(shorten_shared(&value).value(), source);
    fn require_thread_safe<T: Send + Sync>() {}
    require_thread_safe::<CovariantText<'_>>();
}

#[test]
fn contravariance_can_lengthen_consumer_lifetimes() {
    init();
    let consumer: ContravariantConsumer<'static> = {
        let scope = String::from("local scope");
        lengthen(ContravariantConsumer::new(&scope))
    };
    assert_eq!(lengthen_shared(&consumer).consume("permanent"), 9);
    fn require_thread_safe<T: Send + Sync>() {}
    require_thread_safe::<ContravariantConsumer<'_>>();
}

#[test]
fn mixed_variance_preserves_interior_mutation() {
    init();
    let source = String::from("read");
    let replacement = String::from("new slot");
    let value = MixedVariance::new(&source, "scope", "slot", &source);
    let value = mixed_second_covariant(mixed_contravariant(mixed_covariant(value)));
    value.replace(&replacement);
    assert_eq!(value.slot_len(), replacement.len());
    assert_eq!(value.read(), source);
    assert_eq!(value.retained(), source);
    assert_eq!(value.consume("data"), 4);
    fn require_send<T: Send>() {}
    require_send::<MixedVariance<'_, '_, '_, '_>>();
}
