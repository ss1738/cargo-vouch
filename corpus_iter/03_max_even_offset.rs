fn max_even_offset(nums: Vec<i32>) -> Option<i32> {
    nums.iter().enumerate().filter(|&(_, &x)| x % 2 == 0).max_by_key(|&(i, _)| i).map(|(i, _)| i as i32)
}
