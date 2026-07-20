fn increment_option(opt: Option<i32>) -> Option<i32> {
    opt.map(|x| x + 1)
}
