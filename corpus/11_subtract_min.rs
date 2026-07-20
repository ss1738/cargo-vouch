fn subtract_min(numbers: Vec<i32>) -> i32 {
    let min = *numbers.iter().min().unwrap();
    numbers.iter().map(|x| x - min).sum()
}
