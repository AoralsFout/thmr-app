fn main() {
    let native = std::path::Path::new("../../native");
    println!("cargo:rerun-if-changed={}/mp3_decoder.cc", native.display());
    println!(
        "cargo:rerun-if-changed={}/vendor/minimp3/minimp3.h",
        native.display()
    );
    cc::Build::new()
        .cpp(true)
        .file(native.join("mp3_decoder.cc"))
        .include(native.join("vendor/minimp3"))
        .flag_if_supported("-std=c++14")
        .flag_if_supported("/std:c++14")
        .warnings(false)
        .compile("thmr_host_mp3");
}
