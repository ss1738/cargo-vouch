fn sum_of_squares(nums: Vec<i32>) -> i32 {
    nums.iter().map(|x| x * x).sum()
}
