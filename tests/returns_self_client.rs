use api::Counter;

#[test]
fn returns_self_keeps_receiver() {
    init();
    let mut counter = Counter::new(1);
    let address = &raw const counter;
    let receiver = counter.add(2).add(3).try_add(4).unwrap();
    assert!(std::ptr::eq(receiver, address));
    let receiver = receiver.peek().try_peek().unwrap();
    assert!(std::ptr::eq(receiver, address));
    assert_eq!(receiver.value(), 10);
    assert!(counter.try_add(-1).is_err());
    assert_eq!(counter.value(), 10);
    assert!(Counter::new(-1).try_peek().is_err());
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
