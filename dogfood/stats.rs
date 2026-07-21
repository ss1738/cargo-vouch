// Numeric summary helpers over a slice of samples.
fn total(xs: &[i32]) -> i32 {
    xs.iter().sum()
}

fn mean(xs: &[i32]) -> i32 {
    xs.iter().sum::<i32>() / xs.len() as i32
}

fn maximum(xs: &[i32]) -> i32 {
    *xs.iter().max().unwrap()
}

fn minimum(xs: &[i32]) -> i32 {
    *xs.iter().min().unwrap()
}

fn spread(xs: &[i32]) -> i32 {
    maximum(xs) - minimum(xs)
}

fn abs_total(xs: &[i32]) -> i32 {
    xs.iter().map(|x| x.abs()).sum()
}
