fn dot_index(a: Vec<i32>, b: Vec<i32>) -> i32 {
    let mut s = 0;
    for i in 0..a.len() {
        s += a[i] * b[i];
    }
    s
}
