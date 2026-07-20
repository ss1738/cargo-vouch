fn increment_all(numbers: Vec<i32>) -> Vec<i32> {
    numbers.into_iter().map(|x| x + 1).collect()
}
