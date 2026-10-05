fn main() {
    // The pinned adapter version, so an installed app fetches the same one `pnpm install` does.
    println!("cargo:rerun-if-changed=../package.json");
    let package: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string("../package.json").unwrap()).unwrap();
    let version = package["dependencies"]["@agentclientprotocol/claude-agent-acp"]
        .as_str()
        .expect("package.json pins @agentclientprotocol/claude-agent-acp");
    println!("cargo:rustc-env=ORCHARD_ACP_VERSION={version}");
    tauri_build::build()
}
