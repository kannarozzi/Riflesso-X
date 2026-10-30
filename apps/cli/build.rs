//! Embeds the icon (resource 32512, which the mirroring window loads) and the
//! version information. The resource script is written here so it can carry
//! the version.

use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    // Executable resources are a Windows thing; on Linux the window icon is a PNG.
    if env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let numbers: Vec<&str> = version.split(['.', '-']).take(3).collect();
    let numeric = format!("{},0", numbers.join(","));
    let icon = here.join("../desktop/icons/icon.ico").display().to_string().replace('\\', "/");
    let rc = format!(
        r#"32512 ICON "{icon}"
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
      VALUE "FileDescription", "Riflesso X command line"
      VALUE "FileVersion", "{version}"
      VALUE "LegalCopyright", "GPL-3.0-or-later"
      VALUE "OriginalFilename", "riflesso.exe"
      VALUE "ProductName", "Riflesso X"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let rc_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("riflesso.rc");
    fs::write(&rc_path, rc).unwrap();
    embed_resource::compile(&rc_path, embed_resource::NONE).manifest_optional().unwrap();
}
