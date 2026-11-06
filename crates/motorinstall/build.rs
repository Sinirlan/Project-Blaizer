// Windows: the .exe carries the app icon (assets/icon/tools/motorinstall/motorinstall.ico) and its name in the file properties.
// Other targets: nothing to do (the window icon is set at run time, see motorview::app_icon).
fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon/tools/motorinstall/motorinstall.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icon/tools/motorinstall/motorinstall.ico");
        res.set("ProductName", "Motorhead installer");
        res.set("FileDescription", "Motorhead installer");
        if let Err(e) = res.compile() {
            println!("cargo:warning=app icon not embedded: {e}");
        }
    }
}
