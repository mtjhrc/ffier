use ffier_test_lib_via_cdylib as api;

fn main() {
    let error = api::TestErrorGadgetErrorData::new(api::Gadget::new(76));
    let field = error.field_0();
    let _second_owner = api::TestErrorGadgetErrorData::new(*field);
}
