//! Embeds assets/marky.ico in marky.exe.
//!
//! GPUI's `windows-manifest` feature links its own resource for the manifest; this adds the icon
//! group, which Explorer, the taskbar and shortcuts read. The installer uses the same .ico, so the
//! two cannot drift. The manifest is GPUI's, so none is required here.

fn main() {
    println!("cargo:rerun-if-changed=assets/marky.rc");
    println!("cargo:rerun-if-changed=assets/marky.ico");
    embed_resource::compile("assets/marky.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("failed to embed assets/marky.ico");
}
