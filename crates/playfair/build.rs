fn main() {
    let sources = [
        "csrc/playfair.c",
        "csrc/omg_hax.c",
        "csrc/hand_garble.c",
        "csrc/modified_md5.c",
        "csrc/sap_hash.c",
    ];
    for src in &sources {
        println!("cargo:rerun-if-changed={src}");
    }
    cc::Build::new()
        .files(sources)
        .include("csrc")
        .warnings(false)
        .compile("playfair");
}
