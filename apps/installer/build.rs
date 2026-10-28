//! Embeds the app's executable, compressed, and the installer's icon, version
//! information and manifest. Build the app first (`npm run release` does):
//! without it the installer still builds, but can only report the problem.

use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());

    let app = here.join("../../target/release/riflesso-x.exe");
    println!("cargo:rerun-if-changed={}", app.display());
    let payload = match fs::read(&app) {
        Ok(exe) => zstd::encode_all(&exe[..], 19).expect("compressing the app"),
        Err(_) => Vec::new(),
    };
    fs::write(out.join("app.zst"), payload).unwrap();

    // The resource script is written here so it can carry the version.
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let numbers: Vec<&str> = version.split(['.', '-']).take(3).collect();
    let numeric = format!("{},0", numbers.join(","));
    let path = |p: &str| here.join(p).display().to_string().replace('\\', "/");
    let rc = format!(
        r#"1 ICON "{icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "kannarozzi"
      VALUE "FileDescription", "Riflesso X Setup"
      VALUE "FileVersion", "{version}"
      VALUE "LegalCopyright", "GPL-3.0-or-later"
      VALUE "OriginalFilename", "riflesso-setup.exe"
      VALUE "ProductName", "Riflesso X"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = path("../desktop/icons/icon.ico"),
        manifest = path("setup.manifest"),
    );
    let rc_path = out.join("setup.rc");
    fs::write(&rc_path, rc).unwrap();
    println!("cargo:rerun-if-changed=setup.manifest");
    embed_resource::compile(&rc_path, embed_resource::NONE).manifest_required().unwrap();
}
