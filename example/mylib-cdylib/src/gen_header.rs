mylib::__ffier_mylib_library!(ffier_bridge_macros::generate);

fn main() {
    print!("{}", __ffier_header("MYLIB_H").build());
}
