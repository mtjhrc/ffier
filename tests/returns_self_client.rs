use api::Counter;

#[test]
fn self_from_borrowed_receiver_is_a_new_object() {
    init();
    let mut counter = Counter::new(5);
    let copy = counter.duplicate();
    let taken = counter.take();
    assert_eq!((copy.value(), taken.value(), counter.value()), (5, 5, 0));
    assert_eq!(copy.try_duplicate().unwrap().value(), 5);
    assert!(Counter::new(-1).try_duplicate().is_err());
}
