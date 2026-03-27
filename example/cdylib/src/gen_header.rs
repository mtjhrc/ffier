use example_lib::CalcResult;
use example_lib::MyCalculator;

example_lib::my_calculator_ffier!(MyCalculator);
example_lib::calc_result_ffier!(CalcResult);
example_lib::calc_error_error_ffier!("ex");

fn main() {
    let header = ffier::HeaderBuilder::new("EX_H")
        .add(ex_calc_error__header())
        .add(ex_calc_result__header())
        .add(ex_my_calculator__header())
        .build();
    print!("{header}");
}
