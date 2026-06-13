use std::fs;
use std::path::PathBuf;

// rust-embed resolves `#[folder = "..."]` at compile time and fails the build if
// the folder is missing. The admin SPA build output (admin-ui/dist) is gitignored,
// so a fresh checkout or CI run may not have it. Guarantee the embed folder always
// exists with at least a placeholder index.html so the crate always compiles; the
// real SPA, when present, simply overrides the placeholder file.
fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dist = manifest_dir.join("../../admin-ui/dist");

    if !dist.join("index.html").exists() {
        fs::create_dir_all(&dist).expect("create admin-ui/dist placeholder dir");
        let placeholder = "<!doctype html><html lang=\"en\"><head><meta charset=\"UTF-8\">\
<title>Loontail Admin</title></head><body><div id=\"root\">\
Admin UI build is not present. Run `npm run build` in admin-ui/.\
</div></body></html>";
        fs::write(dist.join("index.html"), placeholder).expect("write placeholder index.html");
    }

    println!("cargo:rerun-if-changed=../../admin-ui/dist");
}
