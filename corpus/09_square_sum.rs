fn square_sum(numbers: Vec<i32>) -> i32 {
    numbers.iter().map(|x| x * x).sum()
}
