fn main() {
    slint_build::compile("ui/app.slint").expect("failed to compile ui/app.slint");

    // Icon, version information and manifest for the Windows executable.
    // Well-formed metadata also helps with antivirus/SmartScreen reputation.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/icon.ico");
        println!("cargo:rerun-if-changed=assets/xbgg.manifest");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set_manifest_file("assets/xbgg.manifest")
            .set("ProductName", "XBGG")
            .set("FileDescription", "XBGG - console mode for Windows gaming")
            .set("CompanyName", "PUTTIPONG-Boss")
            .set("LegalCopyright", "Copyright (c) 2026 PUTTIPONG-Boss. MIT License.")
            .set("OriginalFilename", "xbgg.exe")
            .set("InternalName", "xbgg");
        res.compile().expect("failed to embed Windows resources");
    }
}
