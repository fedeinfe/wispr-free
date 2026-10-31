//! The parts of `cargo xtask bridge` that decide things. The real build runs by hand: it takes
//! minutes and needs Xcode's tools.

use xtask::bridge::{link_list, modulemap, unlisted};

const RUSTC: &str = "\
note: Link against the following native artifacts when linking against this static library.
note: native-static-libs: -framework CoreFoundation -framework AppKit -framework CoreFoundation \
-lSystem -lobjc -framework Foundation -liconv -lSystem -lc -lm
    Finished `release` profile [optimized] target(s) in 39.55s
";

#[test]
fn every_namespace_gets_its_own_module() {
    assert_eq!(
        modulemap(),
        "module wfshellFFI { header \"wfshellFFI.h\" export * }\n\
         module wfsettingsFFI { header \"wfsettingsFFI.h\" export * }\n"
    );
}

#[test]
fn the_link_list_comes_from_rustc_without_repeats_or_what_every_program_links() {
    assert_eq!(
        link_list(RUSTC).unwrap(),
        ["AppKit", "CoreFoundation", "Foundation", "iconv"]
    );
    assert_eq!(link_list("Finished"), None);
}

#[test]
fn a_framework_the_package_doesnt_list_is_caught() {
    let package = r#"let frameworks = ["AppKit", "Foundation"]
let libraries = ["iconv"]"#;
    let needed = ["AppKit".to_owned(), "Metal".to_owned(), "iconv".to_owned()];

    assert_eq!(unlisted(&needed, package), ["Metal"]);
}
