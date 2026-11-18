fn main() {
    // Debug info lets the UI tests (src/ui_tests.rs) find elements by id. Release builds skip it.
    let debug = std::env::var("PROFILE").as_deref() != Ok("release");
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug);
    slint_build::compile_with_config("ui/app.slint", config).expect("compile ui/app.slint");
}
