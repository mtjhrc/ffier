use api::Counter;

#[test]
fn returns_self_chains_on_the_receiver() {
    init();
    let mut counter = Counter::new(1);
    let receiver: *const Counter = &counter;
    assert!(std::ptr::eq(counter.add(2).add(3), receiver));
    assert!(std::ptr::eq(counter.peek(), receiver));
    let chained = counter.try_add(4).unwrap();
    assert!(std::ptr::eq(&*chained, receiver));
    assert_eq!(chained.value(), 10);
    assert!(counter.try_add(-1).is_err());
    assert_eq!(counter.value(), 10);
}

#[test]
fn self_from_borrowed_receiver_is_a_new_object() {
    init();
    let mut counter = Counter::new(5);
    let mut copy = counter.duplicate();
    copy.add(1);
    assert_eq!((counter.value(), copy.value()), (5, 6));
    let taken = counter.take();
    assert_eq!((taken.value(), counter.value()), (5, 0));
    assert_eq!(copy.try_duplicate().unwrap().value(), 6);
    assert!(Counter::new(-1).try_duplicate().is_err());
}
