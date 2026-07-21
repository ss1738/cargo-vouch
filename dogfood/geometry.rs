// Small integer-geometry helpers.
fn manhattan(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs() + (a.1 - b.1).abs()
}

fn rect_area(w: i32, h: i32) -> i32 {
    w * h
}

fn midpoint_x(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 + b.0) / 2
}

fn scale(v: (i32, i32), factor: i32) -> i32 {
    v.0 * factor + v.1 * factor
}
