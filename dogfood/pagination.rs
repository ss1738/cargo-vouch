// Pagination arithmetic for a list UI.
fn page_count(total: i32, per_page: i32) -> i32 {
    (total + per_page - 1) / per_page
}

fn offset(page: i32, per_page: i32) -> i32 {
    page * per_page
}

fn clamp_page(page: i32, max_page: i32) -> i32 {
    if page > max_page {
        max_page
    } else if page < 0 {
        0
    } else {
        page
    }
}

fn items_on_page(total: i32, page: i32, per_page: i32) -> i32 {
    let start = page * per_page;
    let remaining = total - start;
    if remaining < per_page {
        remaining
    } else {
        per_page
    }
}
