fn main() {
    // Debug info lets the UI tests (src/ui_tests.rs) find elements by id. Release builds skip it.
    let debug = std::env::var("PROFILE").as_deref() != Ok("release");
    // The `.po` catalogues under translations/<lang>/LC_MESSAGES/kelma.po are compiled in, so the
    // app needs no gettext at run time (plan X1.2). `@tr("...")` strings keep their English
    // source text until a language is selected (src/language.rs). The messages have no
    // per-component context, so a string used on several screens is translated once.
    let config = slint_build::CompilerConfiguration::new()
        .with_debug_info(debug)
        .with_bundled_translations("translations")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app.slint", config).expect("compile ui/app.slint");
}
