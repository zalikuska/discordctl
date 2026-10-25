//! Метаданные exe: версия, описание и манифест. Файл без них антивирусы оценивают строже.

fn main() {
    println!("cargo:rerun-if-changed=../../resources/app.manifest");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }
    winresource::WindowsResource::new()
        .set("FileDescription", "discordctl command-line tool")
        .set("ProductName", "discordctl")
        .set("CompanyName", "discordctl contributors")
        .set("LegalCopyright", "MIT License")
        .set("OriginalFilename", "discordctl.exe")
        .set_manifest_file("../../resources/app.manifest")
        .compile()
        .expect("embedding Windows resources");
}
