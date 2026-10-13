use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=resources/ninja-master.ico");
    println!("cargo:rerun-if-env-changed=WINDRES");
    println!("cargo:rerun-if-env-changed=RC");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let icon = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("resources/ninja-master.ico")
        .to_string_lossy()
        .replace('\\', "/");
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let numeric = ["MAJOR", "MINOR", "PATCH"]
        .map(|part| env::var(format!("CARGO_PKG_VERSION_{part}")).unwrap())
        .join(",");
    let source = format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numeric},0
PRODUCTVERSION {numeric},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0
FILEOS 0x40004L
FILETYPE 1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "FileDescription", "Ninja Master native port\0"
      VALUE "FileVersion", "{version}\0"
      VALUE "InternalName", "ninja-master-native\0"
      VALUE "OriginalFilename", "ninja-master-native.exe\0"
      VALUE "ProductName", "Ninja Master\0"
      VALUE "ProductVersion", "{version}\0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#
    );
    let rc = out.join("ninja-master.rc");
    fs::write(&rc, source).expect("write Windows resources");
    let gnu = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu");
    let object = out.join(if gnu {
        "ninja-master.o"
    } else {
        "ninja-master.res"
    });
    let mut compiler = if gnu {
        let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
        let prefix = match arch.as_str() {
            "x86_64" => "x86_64",
            "x86" => "i686",
            _ => panic!("set WINDRES for the Windows target architecture {arch}"),
        };
        let mut command = Command::new(
            env::var("WINDRES").unwrap_or_else(|_| format!("{prefix}-w64-mingw32-windres")),
        );
        command
            .args(["-i"])
            .arg(&rc)
            .args(["-O", "coff", "-o"])
            .arg(&object);
        command
    } else {
        let mut command = Command::new(env::var("RC").unwrap_or_else(|_| "rc.exe".into()));
        command
            .arg("/nologo")
            .arg(format!("/fo{}", object.display()))
            .arg(&rc);
        command
    };
    assert!(
        compiler
            .status()
            .expect("Windows resource compiler (WINDRES or RC) is required")
            .success(),
        "Windows resource compilation failed"
    );
    println!(
        "cargo:rustc-link-arg-bin=ninja-master-native={}",
        object.display()
    );
}
