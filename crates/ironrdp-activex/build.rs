#[cfg(not(target_os = "windows"))]
use other::main_stub;
#[cfg(target_os = "windows")]
use win::main_stub;

fn main() {
    main_stub()
}

#[cfg(target_os = "windows")]
mod win {
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    const COMPANY_NAME: &str = "Devolutions Inc.";
    const FILE_DESCRIPTION: &str = "IronRDP ActiveX Control";
    const INTERNAL_NAME: &str = "ironrdpax.dll";
    const PRODUCT_NAME: &str = "IronRDP ActiveX";

    fn version_number() -> String {
        let mut version =
            env::var("CARGO_PKG_VERSION").expect("failed to fetch `CARGO_PKG_VERSION` environment variable");
        version.push_str(".0");
        version
    }

    fn version_resource() -> String {
        let version_number = version_number();
        let version_commas = version_number.replace('.', ",");
        let copyright = format!("Copyright (c) 2026 {COMPANY_NAME}");

        format!(
            r#"#include <winresrc.h>
VS_VERSION_INFO VERSIONINFO
    FILEVERSION {version_commas}
    PRODUCTVERSION {version_commas}
    FILEFLAGSMASK 0x3fL
#ifdef _DEBUG
    FILEFLAGS 0x1L
#else
    FILEFLAGS 0x0L
#endif
    FILEOS 0x40004L
    FILETYPE 0x2L
    FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "CompanyName", "{COMPANY_NAME}"
            VALUE "FileDescription", "{FILE_DESCRIPTION}"
            VALUE "FileVersion", "{version_number}"
            VALUE "InternalName", "{INTERNAL_NAME}"
            VALUE "LegalCopyright", "{copyright}"
            VALUE "OriginalFilename", "{INTERNAL_NAME}"
            VALUE "ProductName", "{PRODUCT_NAME}"
            VALUE "ProductVersion", "{version_number}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
        )
    }

    pub(crate) fn main_stub() {
        let out_dir = PathBuf::from(env::var("OUT_DIR").expect("failed to fetch `OUT_DIR` environment variable"));
        let resource_path = out_dir.join("ironrdpax-version.rc");

        fs::write(&resource_path, version_resource()).expect("failed to write the Windows version resource");
        embed_resource::compile(resource_path, embed_resource::NONE)
            .manifest_required()
            .expect("failed to compile the Windows version resource");
    }
}

#[cfg(not(target_os = "windows"))]
mod other {
    pub(crate) fn main_stub() {}
}
