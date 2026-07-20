fn average(numbers: Vec<i32>) -> f64 {
    let sum: i32 = numbers.iter().sum();
    sum as f64 / numbers.len() as f64
}
