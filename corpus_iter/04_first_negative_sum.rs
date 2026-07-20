fn first_negative_sum(nums: Vec<i32>) -> i32 {
    nums.iter().scan(0, |state, &x| {
        *state += x;
        if *state < 0 {
            Some(*state)
        } else {
            None
        }
    }).next().unwrap_or(0)
}
