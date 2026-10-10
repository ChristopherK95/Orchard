fn main() {
    // The pinned adapter version, so an installed app fetches the same one `pnpm install` does.
    println!("cargo:rerun-if-changed=../package.json");
    let package: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string("../package.json").unwrap()).unwrap();
    let version = package["dependencies"]["@agentclientprotocol/claude-agent-acp"]
        .as_str()
        .expect("package.json pins @agentclientprotocol/claude-agent-acp");
    println!("cargo:rustc-env=ORCHARD_ACP_VERSION={version}");
    // The main thread's stack as on Linux (8 MB) rather than Windows' 1 MB: a command's future is
    // built there before it's spawned, and opening a workspace overflowed it.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/STACK:8388608");
    }
    tauri_build::build()
}
