use super::*;

#[test]
fn ffi_is_usable_before_an_app_and_keeps_handles_exact_past_javascript_integer_precision() {
    let mut host = BrowserConnections::new();
    host.registry.next_incarnation = 9_007_199_254_740_992;
    let old = host.open();
    let current = host.open();
    assert_eq!(old, "9007199254740992");
    assert_eq!(current, "9007199254740993");
    assert_eq!(host.bind(&old, "crew"), r#"{"ok":true,"previous":null}"#);
    assert_eq!(
        host.bind(&current, "crew"),
        r#"{"ok":true,"previous":"9007199254740992"}"#
    );
    assert_eq!(host.sender(&old), None);
    assert_eq!(host.close(&old), None);
    assert_eq!(host.sender(&current).as_deref(), Some("crew"));
    assert_eq!(host.recipients("token:crew"), r#"["9007199254740993"]"#);
    assert_eq!(host.recipients("except:crew"), "[]");
    assert_eq!(host.recipients("unknown"), "[]");
    assert_eq!(
        host.bind(&current, "other"),
        r#"{"code":"invalid-token","ok":false}"#
    );
    assert_eq!(host.close(&current).as_deref(), Some("crew"));
    assert_eq!(host.recipients("all"), "[]");
}

#[test]
fn ffi_does_not_accept_aliased_or_invalid_handles() {
    let mut host = BrowserConnections::new();
    let handle = host.open();
    assert_eq!(handle, "0");
    for alias in ["00", "+0", "-0", "", "not-a-handle", "18446744073709551616"] {
        assert_eq!(
            host.bind(alias, "crew"),
            r#"{"code":"invalid-token","ok":false}"#
        );
        assert_eq!(host.sender(alias), None);
        assert_eq!(host.close(alias), None);
    }
    assert_eq!(host.bind(&handle, "crew"), r#"{"ok":true,"previous":null}"#);
    assert_eq!(host.sender(&handle).as_deref(), Some("crew"));
}
