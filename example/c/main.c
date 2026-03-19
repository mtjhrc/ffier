#include <stdio.h>
#include <assert.h>
#include "mycalculator.h"

int main(void) {
    MyCalculatorHandle calc = mycalculator_create();

    /* Plain returns */
    printf("add(3, 4) = %d\n", mycalculator_add(calc, 3, 4));
    printf("multiply(5, 6) = %d\n", mycalculator_multiply(calc, 5, 6));
    printf("negate(42) = %ld\n", mycalculator_negate(calc, 42));
    printf("is_positive(-1) = %d\n", mycalculator_is_positive(calc, -1));
    mycalculator_set_precision(calc, 64);

    /* Result<i32, CalcError> — success */
    int32_t result;
    CalcError err = mycalculator_divide(calc, 10, 3, &result);
    assert(err.code == 0);
    printf("divide(10, 3) = %d\n", result);

    /* Result<i32, CalcError> — error with static message */
    err = mycalculator_divide(calc, 10, 0, &result);
    assert(err.code == CALC_ERROR_DIVISION_BY_ZERO);
    printf("divide(10, 0) = error %lu: %s\n", err.code, calc_error_message(&err));
    ffier_error_free((FfierError*)&err);

    /* checked_add — overflow */
    err = mycalculator_checked_add(calc, 2147483647, 1, &result);
    assert(err.code == CALC_ERROR_OVERFLOW);
    printf("checked_add(INT_MAX, 1) = error %lu: %s\n", err.code, calc_error_message(&err));
    ffier_error_free((FfierError*)&err);

    /* Result<(), CalcError> — no output param */
    err = mycalculator_validate(calc);
    assert(err.code == 0);
    printf("validate() = ok\n");

    mycalculator_destroy(calc);
    printf("All C tests passed!\n");
    return 0;
}
