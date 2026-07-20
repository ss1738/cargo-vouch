fn double_in_place(xs: &mut [i32]) {
    for i in 0..xs.len() {
        xs[i] = xs[i] * 2;
    }
}
