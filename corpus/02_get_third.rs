fn get_third(items: Vec<i32>) -> Option<i32> {
    if items.len() >= 3 {
        Some(items[2])
    } else {
        None
    }
}
