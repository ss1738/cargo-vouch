fn vec_mean(vec: Vec<i32>) -> Option<f64> {
    if vec.is_empty() {
        None
    } else {
        Some(vec.iter().sum::<i32>() as f64 / vec.len() as f64)
    }
}
