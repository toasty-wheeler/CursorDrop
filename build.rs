fn main() {
    // unrar's bundled C++ uses the CryptoAPI for RAR encryption support.
    println!("cargo:rustc-link-lib=advapi32");
}
