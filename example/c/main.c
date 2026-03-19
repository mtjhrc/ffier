#include <stdio.h>
#include <assert.h>
#include "mycalculator.h"

/* CalcError codes (0 = success) */
#define CALC_OK             0
#define CALC_DIVISION_BY_ZERO 1
#define CALC_OVERFLOW       2

int main(void) {
    MyCalculatorHandle calc = mycalculator_create();

    /* Plain returns */
    printf("add(3, 4) = %d\n", mycalculator_add(calc, 3, 4));
    printf("multiply(5, 6) = %d\n", mycalculator_multiply(calc, 5, 6));
    printf("negate(42) = %ld\n", mycalculator_negate(calc, 42));
    printf("is_positive(-1) = %d\n", mycalculator_is_positive(calc, -1));
    mycalculator_set_precision(calc, 64);

    /* Result<i32, CalcError> — success case */
    int32_t result;
    int32_t err = mycalculator_divide(calc, 10, 3, &result);
    assert(err == CALC_OK);
    printf("divide(10, 3) = %d\n", result);

    /* Result<i32, CalcError> — error case */
    err = mycalculator_divide(calc, 10, 0, &result);
    assert(err == CALC_DIVISION_BY_ZERO);
    printf("divide(10, 0) = error %d (division by zero)\n", err);

    /* checked_add — success */
    err = mycalculator_checked_add(calc, 100, 200, &result);
    assert(err == CALC_OK);
    printf("checked_add(100, 200) = %d\n", result);

    /* checked_add — overflow */
    err = mycalculator_checked_add(calc, 2147483647, 1, &result);
    assert(err == CALC_OVERFLOW);
    printf("checked_add(INT_MAX, 1) = error %d (overflow)\n", err);

    /* Result<(), CalcError> — no output param */
    err = mycalculator_validate(calc);
    assert(err == CALC_OK);
    printf("validate() = ok\n");

    mycalculator_destroy(calc);
    printf("All C tests passed!\n");
    return 0;
}
