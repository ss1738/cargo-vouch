fn add_scalar(xs: &[i32], k: i32) -> Vec<i32> {
    xs.iter().map(|x| x + k).collect()
}
