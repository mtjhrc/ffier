use example_lib::MyCalculator;
use example_lib::CalcResult;

example_lib::mycalculator_ffier!(MyCalculator);
example_lib::calcresult_ffier!(CalcResult);

fn main() {
    // CalcResult header first since MyCalculator references it
    print!("{}", ex_calcresult__header());
    println!();
    print!("{}", ex_mycalculator__header());
}
