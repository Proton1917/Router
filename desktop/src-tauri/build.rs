fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "setup_info",
            "choose_config",
            "create_config",
            "open_saved",
            "setup_models",
            "dependency_help",
            "update_installation",
        ]),
    ))
    .expect("无法构建桌面资源");
}
