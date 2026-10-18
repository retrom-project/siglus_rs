use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=SIGLUS_DEFAULT_FONT_PATH");
    let path = std::env::var_os("SIGLUS_DEFAULT_FONT_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
                .join("../../dist/retrom-inputs/font/NotoSansSC.ttf")
        })
        .canonicalize()
        .expect("run python3 tools/build_retrom_candidate.py <empty-output> to prepare the licensed Noto Sans SC font, or set SIGLUS_DEFAULT_FONT_PATH");
    println!("cargo:rerun-if-changed={}", path.display());
    println!(
        "cargo:rustc-env=SIGLUS_DEFAULT_FONT_PATH={}",
        path.display()
    );
}
