//! Build script — compiles 2 C files for sap_hash + garble.

fn main() {
    let mut build = cc::Build::new();
    build
        .file("src/fairplay/hand_garble.c")
        .file("src/fairplay/sap_hash.c")
        .include("src/fairplay");
    if build.get_compiler().is_like_msvc() {
        build.flag("/wd4100").flag("/wd4018").flag("/wd4244").flag("/wd4267");
    }
    build.compile("playfair");
}
