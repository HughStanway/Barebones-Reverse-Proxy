use barebones_reverse_proxy::utils;

#[test]
fn test_service() {
    assert_eq!(utils::do_something(), "Hello from utils");
}