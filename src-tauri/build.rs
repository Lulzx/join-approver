// The Telegram API ID and hash are compiled in, so nobody using the app has to
// register their own. They come from the environment or from ../.env (never
// committed) rather than from source.
fn main() {
    let env_file = std::path::Path::new("../.env");
    println!("cargo:rerun-if-changed=../.env");
    println!("cargo:rerun-if-env-changed=TG_API_ID");
    println!("cargo:rerun-if-env-changed=TG_API_HASH");

    let from_file = std::fs::read_to_string(env_file).unwrap_or_default();
    let lookup = |key: &str| {
        std::env::var(key).ok().or_else(|| {
            from_file.lines().find_map(|line| {
                let (k, v) = line.split_once('=')?;
                (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
            })
        })
    };
    let id = lookup("TG_API_ID").expect("TG_API_ID is not set (env or ../.env)");
    let hash = lookup("TG_API_HASH").expect("TG_API_HASH is not set (env or ../.env)");
    assert!(id.parse::<i32>().is_ok(), "TG_API_ID must be a number");
    // A generated file, not rustc-env: build logs echo cargo:rustc-env lines.
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("api_keys.rs");
    std::fs::write(
        out,
        format!("pub const API_ID: i32 = {id};\npub const API_HASH: &str = {hash:?};\n"),
    )
    .expect("writing api_keys.rs");

    tauri_build::build()
}
