fn main() {
    println!("cargo:rerun-if-changed=assets/branding/nera.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("assets/branding/nera.ico")
            .set("FileDescription", "Project Nera Desktop Pet")
            .set("ProductName", "Project Nera")
            .set("OriginalFilename", "nera.exe")
            .set(
                "LegalCopyright",
                "Copyright (c) 2026 Sahid Haqqi. All rights reserved.",
            );
        resource
            .compile()
            .expect("Compile Nera Windows resources (Windows SDK rc.exe required)");
        // Scope the GUI subsystem to the release application. Validation
        // examples include main.rs; an inner crate attribute would break them.
        if std::env::var("PROFILE").as_deref() == Ok("release") {
            println!("cargo:rustc-link-arg-bin=nera=/SUBSYSTEM:WINDOWS");
            println!("cargo:rustc-link-arg-bin=nera=/ENTRY:mainCRTStartup");
        }
    }
}
