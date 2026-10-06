use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=res/app.rc");
    println!("cargo:rerun-if-changed=res/app.manifest");
    println!("cargo:rerun-if-changed=res/icons");
    println!("cargo:rerun-if-changed=third_party/needle/libneedle3.dll");

    let version = env!("CARGO_PKG_VERSION");
    let mut parts: Vec<u32> = version
        .split(['.', '-'])
        .filter_map(|p| p.parse().ok())
        .collect();
    parts.resize(4, 0);
    let commas = format!("{},{},{},{}", parts[0], parts[1], parts[2], parts[3]);

    // Only the GUI executable gets the manifest/icon/version resource.
    embed_resource::compile_for(
        "res/app.rc",
        ["WhistleType"],
        [
            format!("WT_VERSION_COMMAS={commas}"),
            format!("WT_VERSION_STR=\"{version}\""),
        ],
    )
    .manifest_required()
    .unwrap();

    // Development convenience: put the engine DLL next to the built executables (target/<profile>/).
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dll = manifest.join("third_party").join("needle").join("libneedle3.dll");
    if dll.exists() {
        let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
        // OUT_DIR = target/<profile>/build/<pkg>/out
        if let Some(profile_dir) = out.ancestors().nth(3) {
            let _ = std::fs::copy(&dll, profile_dir.join("libneedle3.dll"));
        }
    } else {
        println!("cargo:warning=third_party/needle/libneedle3.dll missing - run scripts/fetch-engine.ps1");
    }

    // The whisper.cpp CPU runtime pack goes to target/<profile>/whisper-cpu (like the installer layout).
    let wsrc = manifest.join("third_party").join("whisper-cpu");
    println!("cargo:rerun-if-changed=third_party/whisper-cpu");
    if wsrc.join("whisper.dll").exists() {
        let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
        if let Some(profile_dir) = out.ancestors().nth(3) {
            let dst = profile_dir.join("whisper-cpu");
            let _ = std::fs::create_dir_all(&dst);
            if let Ok(rd) = std::fs::read_dir(&wsrc) {
                for e in rd.flatten() {
                    let target = dst.join(e.file_name());
                    let same = std::fs::metadata(&target).ok().map(|m| m.len()) == e.metadata().ok().map(|m| m.len());
                    if !same {
                        let _ = std::fs::copy(e.path(), target);
                    }
                }
            }
        }
    } else {
        println!("cargo:warning=third_party/whisper-cpu missing - run scripts/fetch-engine.ps1");
    }
}
