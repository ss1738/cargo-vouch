// pagination.rs with the guards cargo-vouch's report asked for.
fn page_count(total: i32, per_page: i32) -> i32 {
    if per_page <= 0 {
        return 0;
    }
    total.saturating_add(per_page - 1) / per_page
}

fn offset(page: i32, per_page: i32) -> i32 {
    page.saturating_mul(per_page)
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
