#[test]
fn javascript_bridge_contracts() {
    let status = std::process::Command::new("node")
        .args(["--test", "tests/bridge.test.cjs"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .expect("Node.js is required to exercise the Comix browser bridge");
    assert!(status.success(), "Comix browser bridge behavior regressed");
}
