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
    use std::fs::File;
    use std::io::Write as _;

    fn generate_version_rc() -> String {
        let output_name = "DevolutionsIronRdp";
        let filename = format!("{output_name}.dll");
        let company_name = "Devolutions Inc.";
        let legal_copyright = format!("Copyright 2019-2024 {company_name}");

        let mut cargo_version =
            env::var("CARGO_PKG_VERSION").expect("failed to fetch `CARGO_PKG_VERSION` environment variable");
        cargo_version.push_str(".0");

        let version_number = cargo_version;
        let version_commas = version_number.replace('.', ",");
        let file_description = output_name;
        let file_version = version_number.clone();
        let internal_name = filename.clone();
        let original_filename = filename;
        let product_name = output_name;
        let product_version = version_number;
        let vs_file_version = version_commas.clone();
        let vs_product_version = version_commas;

        let version_rc = format!(
            r#"#include <winresrc.h>
VS_VERSION_INFO VERSIONINFO
    FILEVERSION {vs_file_version}
    PRODUCTVERSION {vs_product_version}
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
            VALUE "CompanyName", "{company_name}"
            VALUE "FileDescription", "{file_description}"
            VALUE "FileVersion", "{file_version}"
            VALUE "InternalName", "{internal_name}"
            VALUE "LegalCopyright", "{legal_copyright}"
            VALUE "OriginalFilename", "{original_filename}"
            VALUE "ProductName", "{product_name}"
            VALUE "ProductVersion", "{product_version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
        );

        version_rc
    }

    pub(crate) fn main_stub() {
        let out_dir = env::var("OUT_DIR").expect("failed to fetch `OUT_DIR` environment variable");
        let version_rc_file = format!("{out_dir}/version.rc");
        let version_rc_data = generate_version_rc();
        let mut file = File::create(&version_rc_file).expect("failed to create version.rc file");
        file.write_all(version_rc_data.as_bytes())
            .expect("failed to write data to version.rc file");
        embed_resource::compile(&version_rc_file, embed_resource::NONE)
            .manifest_required()
            .expect("failed to compiler the Windows resource file");
    }
}

#[cfg(not(target_os = "windows"))]
mod other {
    pub(crate) fn main_stub() {}
}
