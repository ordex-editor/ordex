use workspace_one::helper_value;

fn main() {
    let _ = helper_value();
    let _ = local_value();
}

fn local_value() -> i32 {
    11
}

/// Adds two numbers.
fn helper_sum(left: i32, right: i32) -> i32 {
    left + right
}
