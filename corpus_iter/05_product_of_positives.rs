fn product_of_positives(nums: Vec<i32>) -> i32 {
    nums.iter().filter(|&&x| x > 0).fold(1, |acc, &x| acc * x)
}
