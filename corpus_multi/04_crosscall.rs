fn helper(x: i32) -> i32 { x + 1 }
fn caller(v: Vec<i32>) -> i32 { helper(v[0]) }
fn safe_caller(x: i32) -> i32 { double(x) }
fn double(x: i32) -> i32 { x.wrapping_mul(2) }
