use example_lib::CalcResult;
use example_lib::MyCalculator;

example_lib::mycalculator_ffier!(MyCalculator);
example_lib::calcresult_ffier!(CalcResult);

fn main() {
    let header = ffier::HeaderBuilder::new("EX_H")
        .add(ex_calcresult__header())
        .add(ex_mycalculator__header())
        .build();
    print!("{header}");
}
