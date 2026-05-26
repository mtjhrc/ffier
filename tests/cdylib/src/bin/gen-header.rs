fn main() {
    let json = std::fs::read_to_string(
        std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("target/ffier-ft.json"),
    )
    .expect("build the cdylib first to generate the schema JSON");
    let lib = ffier_schema::Library::from_json(&json).unwrap();
    let guard = format!("FFIER_{}_H", lib.prefix.to_ascii_uppercase());
    print!("{}", ffier_gen_c_header::generate(&lib, &guard));
}
