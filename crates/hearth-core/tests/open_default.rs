//! `Core::open_default` finds its directories the way earlier builds did,
//! including the overrides. Alone in its file because it sets environment
//! variables for the whole process.

use hearth_core::Core;

#[test]
fn open_default_honours_the_directory_overrides() {
    let base = std::env::temp_dir().join(format!("hearth-default-{}", uuid::Uuid::new_v4().simple()));
    let (config, data) = (base.join("config"), base.join("data"));
    std::env::set_var("HEARTH_CONFIG_DIR", &config);
    std::env::set_var("HEARTH_DATA_DIR", &data);

    // No servers are saved, so the system keychain is never asked for anything.
    let core = Core::open_default(|_| {}).unwrap();
    core.save_settings(serde_json::json!({"theme": "dark"})).unwrap();
    core.close();

    assert!(config.join("config.json").is_file());
    assert!(data.join("hearth.db").is_file());
    let _ = std::fs::remove_dir_all(base);
}
