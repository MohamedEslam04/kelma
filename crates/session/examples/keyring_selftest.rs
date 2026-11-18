//! Stores, reads and deletes a dummy secret to check the Secret Service keyring works.
fn main() {
    let acct = "selftest";
    session::save_refresh_token(acct, "dummy-value").expect("save");
    assert_eq!(session::load_refresh_token(acct).expect("load").as_deref(), Some("dummy-value"));
    session::delete_refresh_token(acct).expect("delete");
    assert_eq!(session::load_refresh_token(acct).expect("load after delete"), None);
    println!("keyring OK");
}
