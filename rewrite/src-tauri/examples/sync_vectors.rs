#[path = "../src/sync_crypto.rs"]
mod sync_crypto;
fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&sync_crypto::test_vectors()).unwrap()
    );
}
