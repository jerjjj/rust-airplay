//! Build script: compiles UxPlay's playfair C library for FairPlay/OmgHax.

fn main() {
    let mut build = cc::Build::new();
    build
        .file("src/fairplay/hand_garble.c")
        .file("src/fairplay/modified_md5_c.c")
        .file("src/fairplay/omg_hax.c")
        .file("src/fairplay/playfair.c")
        .file("src/fairplay/sap_hash.c")
        .include("src/fairplay");

    // Try MSVC first, fall back to GCC
    if build.get_compiler().is_like_msvc() {
        // MSVC: disable C4100, C4018 warnings (unreferenced formal parameter, signed/unsigned mismatch)
        build.flag("/wd4100").flag("/wd4018").flag("/wd4244").flag("/wd4267");
    }
    build.compile("playfair");
    println!("cargo:rerun-if-changed=src/fairplay/hand_garble.c");
    println!("cargo:rerun-if-changed=src/fairplay/modified_md5_c.c");
    println!("cargo:rerun-if-changed=src/fairplay/omg_hax.c");
    println!("cargo:rerun-if-changed=src/fairplay/omg_hax.h");
    println!("cargo:rerun-if-changed=src/fairplay/playfair.c");
    println!("cargo:rerun-if-changed=src/fairplay/sap_hash.c");
}
