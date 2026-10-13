use ffier_test_lib_via_cdylib as api;

fn main() {
    let field;
    {
        let error = api::TestErrorGadgetErrorData::new(api::Gadget::new(76));
        field = error.field_0();
    }
    assert_eq!(field.get(), 76);
}
