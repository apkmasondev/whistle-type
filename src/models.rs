//! Catalogue of the ACCURATE (Whisper) models and of the whisper.cpp runtime packs, plus GPU detection.
//! The FAST model (Whistle) is described in `model.rs`.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::util::sha256_file;
use crate::{log_info, log_warn};

/// Official GGML conversions of OpenAI Whisper (MIT), pinned to an immutable Hugging Face revision.
pub const WHISPER_REPO: &str = "ggerganov/whisper.cpp";
pub const WHISPER_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
pub const WHISPER_LICENSE: &str = "MIT";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// fine on a CPU
    Cpu,
    /// recommended with an NVIDIA GPU
    Gpu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WhisperModel {
    pub id: &'static str,
    pub name: &'static str,
    pub file: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub params_m: u32,
    pub tier: Tier,
    /// 1 (basic) .. 4 (best) - Polish quality, see PERFORMANCE.md for measured WER
    pub polish: u8,
}

pub const WHISPER_MODELS: &[WhisperModel] = &[
    WhisperModel {
        id: "whisper-base",
        name: "Whisper base",
        file: "ggml-base-q5_1.bin",
        size: 59_707_625,
        sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
        params_m: 74,
        tier: Tier::Cpu,
        polish: 1,
    },
    WhisperModel {
        id: "whisper-small",
        name: "Whisper small",
        file: "ggml-small-q5_1.bin",
        size: 190_085_487,
        sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
        params_m: 244,
        tier: Tier::Cpu,
        polish: 2,
    },
    WhisperModel {
        id: "whisper-medium",
        name: "Whisper medium",
        file: "ggml-medium-q5_0.bin",
        size: 539_212_467,
        sha256: "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f",
        params_m: 769,
        tier: Tier::Gpu,
        polish: 3,
    },
    WhisperModel {
        id: "whisper-large-v3-turbo",
        name: "Whisper large-v3-turbo",
        file: "ggml-large-v3-turbo-q5_0.bin",
        size: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        params_m: 809,
        tier: Tier::Gpu,
        polish: 4,
    },
];

pub const DEFAULT_WHISPER_GPU: &str = "whisper-large-v3-turbo";
pub const DEFAULT_WHISPER_CPU: &str = "whisper-small";

pub fn whisper_model(id: &str) -> Option<&'static WhisperModel> {
    WHISPER_MODELS.iter().find(|m| m.id == id)
}

pub fn whisper_models_dir() -> PathBuf {
    crate::paths::models_dir().join("whisper")
}

pub fn whisper_model_path(m: &WhisperModel) -> PathBuf {
    whisper_models_dir().join(m.file)
}

pub fn whisper_url_path(m: &WhisperModel) -> String {
    format!("/{WHISPER_REPO}/resolve/{WHISPER_REVISION}/{}", m.file)
}

/// Installed = present with the right size (the SHA-256 was checked when it was downloaded/imported).
pub fn whisper_installed(m: &WhisperModel) -> bool {
    std::fs::metadata(whisper_model_path(m)).is_ok_and(|md| md.len() == m.size)
}

pub fn installed_whisper_models() -> Vec<&'static WhisperModel> {
    WHISPER_MODELS.iter().filter(|m| whisper_installed(m)).collect()
}

// ------------------------------------------------------------------------------------------------
// whisper.cpp runtime packs
// ------------------------------------------------------------------------------------------------

/// The CPU pack ships with WhistleType (`<app>\whisper-cpu`).
pub fn cpu_runtime_dir() -> PathBuf {
    crate::paths::exe_dir().join("whisper-cpu")
}

/// `<path><suffix>` next to `path` (`with_extension` would cut "whisper-cuda-12.4-b5130" at the last dot).
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Official whisper.cpp release asset with the CUDA 12.4 build (contains NVIDIA cuBLAS/cudart, which NVIDIA allows
/// to redistribute with applications). Downloaded on demand from the Model Manager.
pub struct CudaPack;

impl CudaPack {
    pub const HOST: &'static str = "github.com";
    pub const PATH: &'static str = "/ggml-org/whisper.cpp/releases/download/b5130/whisper-cublas-12.4.0-bin-x64.zip";
    pub const SIZE: u64 = 674_539_285;
    pub const SHA256: &'static str = "af520ddd034d985b55dfeea3e465ed93653ba2aee1a55e865033edc548c272a7";
    /// Files taken from the archive (`Release/<name>`): name, size, SHA-256.
    pub const FILES: &'static [(&'static str, u64, &'static str)] = &[
        ("whisper.dll", 1_311_744, "26e5b494d807e3e94bd4739da8dac579b3d047765d2e5545251d053c5ead1c7b"),
        ("ggml.dll", 60_928, "5ee657aee335f23cea8bfb7e6aeaf42773d31c38198030e51554dcebbfab79e1"),
        ("ggml-base.dll", 672_768, "7fb5900986476a3ff3160b8ec002005d05c4ba4e6aec8c61ba59b254dcac776c"),
        ("ggml-cuda.dll", 544_947_200, "1bb5e1526824f39cccb2a1fba609798e383615ad7e732b7bbf675a3d9949610f"),
        ("cudart64_12.dll", 553_984, "d28e42265da7462162a54da6b7a99ea4fa2caf8139d862bb500db875d0b32dfc"),
        ("cublas64_12.dll", 100_033_536, "e40202fe4223c1cd2d2dce7beec59e1ed61c7801bd827309183be9b50e358f4c"),
        ("cublasLt64_12.dll", 473_551_360, "2a896460bef60ed57ef32b0875812f355a6984e671d638bb632f5e8c1d7a831f"),
        ("ggml-cpu-alderlake.dll", 857_088, "87f839db47b18cbd05f164f4b36d6c720ab1bedae7674cd9f6b7187e3b68850a"),
        ("ggml-cpu-cannonlake.dll", 901_632, "ad48beacb670415421e6d0e38255f9796f803e304de92bbeb3b5de35a2da67cf"),
        ("ggml-cpu-cascadelake.dll", 898_560, "55625077ba07db064c916c11bc5240c3bf50a960eb370cb042a7fbaa4c65af95"),
        ("ggml-cpu-haswell.dll", 858_112, "f87122aa07365e922e22906ddad03073745188d0cbfb6941fbb4d7c1df983945"),
        ("ggml-cpu-icelake.dll", 898_560, "a03e5f9bd16f32e382e21297041306d01bf0db1c2268f0391c853ac443c550cd"),
        ("ggml-cpu-sandybridge.dll", 832_000, "192f02b5407bdb508c982b31d929f7bd14872003e6d5f1c68f2e6f21fd22b4b0"),
        ("ggml-cpu-skylakex.dll", 901_632, "b62ef6e0a36868cec1bf93c9c103a3a8ec0f327810f032b02d3f801e6f0d1dbc"),
        ("ggml-cpu-sse42.dll", 819_200, "e47f1d663d8e92f63d10dc64d57c1307e39acd8542bfe7572f7b0094aa4acb70"),
        ("ggml-cpu-x64.dll", 820_736, "558dd2ed51f0aa40ffec14bdaf2204d2c0c7b46876307733a8c492b5636b5273"),
    ];
    /// The Visual C++ runtime files the whisper.cpp DLLs need, copied from the CPU pack (app-local deployment).
    pub const VC_RUNTIME: &'static [&'static str] = &["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll", "vcomp140.dll"];

    pub fn dir() -> PathBuf {
        crate::paths::local_dir().join("runtimes").join("whisper-cuda-12.4-b5130")
    }

    pub fn installed_bytes() -> u64 {
        Self::FILES.iter().map(|(_, s, _)| s).sum()
    }

    /// Installed = every file present with the right size (hashes are checked when installing).
    pub fn installed() -> bool {
        let d = Self::dir();
        Self::FILES.iter().all(|(n, s, _)| std::fs::metadata(d.join(n)).is_ok_and(|m| m.len() == *s))
            && Self::VC_RUNTIME.iter().all(|n| d.join(n).exists())
    }

    /// Extracts and verifies the needed files from the downloaded archive into the pack folder.
    pub fn install_from_zip(zip_path: &Path) -> Result<(), String> {
        let dest = Self::dir();
        let staging = sibling(&dest, ".part");
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
        let f = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(f).map_err(|e| format!("archive: {e}"))?;
        for (name, size, sha) in Self::FILES {
            let mut entry = zip.by_name(&format!("Release/{name}")).map_err(|e| format!("{name}: {e}"))?;
            let out = staging.join(name);
            let mut w = std::fs::File::create(&out).map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; 1 << 20];
            loop {
                let n = entry.read(&mut buf).map_err(|e| format!("{name}: {e}"))?;
                if n == 0 {
                    break;
                }
                std::io::Write::write_all(&mut w, &buf[..n]).map_err(|e| e.to_string())?;
            }
            drop(w);
            let got_size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
            let got = sha256_file(&out).map_err(|e| e.to_string())?;
            if got_size != *size || got != *sha {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(format!("{name} failed verification"));
            }
        }
        for vc in Self::VC_RUNTIME {
            std::fs::copy(cpu_runtime_dir().join(vc), staging.join(vc)).map_err(|e| format!("{vc}: {e}"))?;
        }
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&staging, &dest).map_err(|e| e.to_string())?;
        log_info!("models: CUDA runtime pack installed in {}", dest.display());
        Ok(())
    }

    /// Removes the pack (fails while its DLLs are loaded - the app then asks for a restart).
    pub fn remove() -> Result<(), String> {
        match std::fs::remove_dir_all(Self::dir()) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn removal_marker() -> PathBuf {
        sibling(&Self::dir(), ".remove")
    }

    /// The pack is in use: remove it at the next start, before any whisper.cpp DLL is loaded.
    pub fn schedule_removal() -> Result<PathBuf, String> {
        let m = Self::removal_marker();
        std::fs::write(&m, b"remove").map_err(|e| e.to_string())?;
        Ok(m)
    }

    /// Removal scheduled by [`CudaPack::schedule_removal`] and not done yet.
    pub fn removal_pending() -> bool {
        Self::removal_marker().exists()
    }

    /// Called at start-up.
    pub fn apply_scheduled_removal() {
        let m = Self::removal_marker();
        if m.exists() {
            match Self::remove() {
                Ok(()) => log_info!("models: CUDA pack removed (scheduled)"),
                Err(e) => log_warn!("models: scheduled removal of the CUDA pack failed: {e}"),
            }
            let _ = std::fs::remove_file(m);
        }
    }
}

/// Runtime folders to try, best first. Only one can be loaded per process.
pub fn runtime_dirs(use_gpu: bool) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if use_gpu && CudaPack::installed() {
        v.push(CudaPack::dir());
    }
    v.push(cpu_runtime_dir());
    v
}

// ------------------------------------------------------------------------------------------------
// GPU detection (DXGI - works before any CUDA code is installed)
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    pub vram_bytes: u64,
    pub nvidia: bool,
}

pub fn adapters() -> Vec<Adapter> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
    let mut out = Vec::new();
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return out };
    let mut i = 0;
    while let Ok(a) = unsafe { factory.EnumAdapters1(i) } {
        i += 1;
        let Ok(d) = (unsafe { a.GetDesc1() }) else { continue };
        if d.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
            continue;
        }
        out.push(Adapter {
            name: crate::util::from_wide(&d.Description),
            vram_bytes: d.DedicatedVideoMemory as u64,
            nvidia: d.VendorId == 0x10DE,
        });
    }
    out
}

/// The NVIDIA GPU WhistleType can use with the CUDA pack, if any.
pub fn nvidia_gpu() -> Option<Adapter> {
    adapters().into_iter().filter(|a| a.nvidia).max_by_key(|a| a.vram_bytes)
}

/// [`nvidia_gpu`], enumerated once per process (the UI asks often; adapters do not come and go in practice).
pub fn nvidia_gpu_cached() -> Option<Adapter> {
    static GPU: std::sync::OnceLock<Option<Adapter>> = std::sync::OnceLock::new();
    GPU.get_or_init(nvidia_gpu).clone()
}

/// Deletes a downloaded Whisper model.
pub fn delete_whisper(m: &WhisperModel) -> Result<(), String> {
    let p = whisper_model_path(m);
    let _ = std::fs::remove_file(sibling(&p, ".part")); // an unfinished download of it, if any
    match std::fs::remove_file(&p) {
        Ok(()) => {
            log_info!("models: deleted {}", m.file);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => {
            log_warn!("models: cannot delete {}: {e}", m.file);
            Err(e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_is_consistent() {
        assert!(whisper_model(DEFAULT_WHISPER_GPU).is_some());
        assert!(whisper_model(DEFAULT_WHISPER_CPU).is_some());
        for m in WHISPER_MODELS {
            assert_eq!(m.sha256.len(), 64);
            assert!(m.file.starts_with("ggml-") && m.file.ends_with(".bin"));
            assert!(whisper_url_path(m).contains(WHISPER_REVISION));
        }
        let mut ids: Vec<_> = WHISPER_MODELS.iter().map(|m| m.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), WHISPER_MODELS.len());
        assert!(CudaPack::FILES.iter().all(|(_, _, h)| h.len() == 64));
        assert!(CudaPack::installed_bytes() > 1_000_000_000);
        assert_eq!(sibling(Path::new(r"C:\x\whisper-cuda-12.4-b5130"), ".remove"), Path::new(r"C:\x\whisper-cuda-12.4-b5130.remove"));
    }
}
