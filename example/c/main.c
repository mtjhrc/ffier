#include <stdio.h>
#include <assert.h>
#include "mycalculator.h"

int main(void) {
    ExMyCalculatorHandle calc = ex_mycalculator_create();

    /* Plain returns */
    printf("add(3, 4) = %d\n", ex_mycalculator_add(calc, 3, 4));
    printf("multiply(5, 6) = %d\n", ex_mycalculator_multiply(calc, 5, 6));
    printf("negate(42) = %ld\n", ex_mycalculator_negate(calc, 42));
    printf("is_positive(-1) = %d\n", ex_mycalculator_is_positive(calc, -1));
    ex_mycalculator_set_precision(calc, 64);

    /* &str return (zero-copy, borrows from object) */
    ExStr name = ex_mycalculator_name(calc);
    printf("name() = %.*s\n", (int)name.len, name.data);

    /* &str param + return (zero-copy round-trip) */
    ExStr greeting = ex_mycalculator_echo(calc, EX_STR("hello from C!"));
    printf("echo() = %.*s\n", (int)greeting.len, greeting.data);

    /* &[u8] return */
    ExBytes data = ex_mycalculator_data(calc);
    printf("data() = %zu bytes\n", data.len);

    /* Result<i32, CalcError> — success */
    int32_t result;
    ExCalcError err = ex_mycalculator_divide(calc, 10, 3, &result);
    assert(err.code == 0);
    printf("divide(10, 3) = %d\n", result);

    /* Result<i32, CalcError> — error */
    err = ex_mycalculator_divide(calc, 10, 0, &result);
    assert(err.code == EX_CALC_ERROR_DIVISION_BY_ZERO);
    printf("divide(10, 0) = error %lu: %s\n", err.code, ex_calc_error_message(&err));
    ex_calc_error_free(&err);

    /* checked_add — overflow */
    err = ex_mycalculator_checked_add(calc, 2147483647, 1, &result);
    assert(err.code == EX_CALC_ERROR_OVERFLOW);
    printf("checked_add(INT_MAX, 1) = error %lu: %s\n", err.code, ex_calc_error_message(&err));
    ex_calc_error_free(&err);

    /* Result<(), CalcError> */
    err = ex_mycalculator_validate(calc);
    assert(err.code == 0);
    printf("validate() = ok\n");

    ex_mycalculator_destroy(calc);
    printf("All C tests passed!\n");
    return 0;
}
