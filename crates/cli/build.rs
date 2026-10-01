// Embed a Windows version resource so `sap-odata.exe` reports its real
// version (Explorer's Details tab, `(Get-Command sap-odata).Version`)
// instead of 0.0.0.0. FileVersion / ProductVersion come from
// CARGO_PKG_VERSION. Best effort: without a resource compiler (e.g. a
// `cargo install` on a machine lacking the Windows SDK's rc.exe) the exe
// just builds without the metadata — never a build failure.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set("FileDescription", "SAP OData Explorer CLI")
        .set("ProductName", "SAP OData Explorer")
        .set("OriginalFilename", "sap-odata.exe")
        .set("LegalCopyright", "MIT License");
    if let Err(e) = res.compile() {
        println!("cargo:warning=skipping Windows version resource: {e}");
    }
}
