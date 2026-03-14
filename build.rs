fn main() {
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file("bridge.cpp")
        .include("runtime")
        .compile("elem_bridge");
}
