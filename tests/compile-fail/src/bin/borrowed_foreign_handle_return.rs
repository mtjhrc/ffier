//! expected-error: ffier: `Owner::item` returns the borrowed handle `&ForeignItem`, which is not supported
//! features: receiver-escapes

use ffier_test_foreign_lib_via_cdylib::ForeignItem;

pub struct Owner(ForeignItem);

#[ffier::export]
impl Owner {
    #[ffier(foreign_return = ::ffier_test_foreign_lib_via_cdylib, c_name = "FlForeignItem")]
    pub fn item(&self) -> &ForeignItem {
        &self.0
    }
}

ffier::library_definition!("ui", library_tag = 1, Owner = 1);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
