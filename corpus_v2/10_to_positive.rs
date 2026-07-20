fn to_positive(opt: Option<i32>) -> Option<i32> {
    opt.filter(|&x| x > 0)
}
