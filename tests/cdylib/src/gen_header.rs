ffier_test_lib::__ffier_ft_library!(ffier_gen_c_macros::generate);

fn main() {
    print!("{}", __ffier_header("FFIER_TEST_H").build());
}
