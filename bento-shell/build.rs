fn main() {
    // gtk4-layer-shell has to come before libwayland-client, which it wraps, or windows silently
    // stay ordinary windows. A binary's own libraries are linked before its dependencies'.
    println!("cargo:rustc-link-lib=dylib=gtk4-layer-shell");
}
