fn filter_and_double_odds(nums: Vec<i32>) -> Vec<i32> {
    nums.into_iter().filter(|&x| x % 2 != 0).map(|x| x * 2).collect()
}
