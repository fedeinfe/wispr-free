//! Ticket 11's done-when: a fresh install on an 8 GB machine recommends Lite and downloads only
//! what Lite needs.

mod support;

use std::sync::atomic::AtomicBool;

use support::{TestServer, payload};
use wf_models::hardware::{CpuArch, CpuFeatures, HardwareInfo};
use wf_models::{Catalog, ModelFile, ModelStatus, ModelStore, Progress, Tier};

fn entry(id: &str, kind: &str, engine: &str, file: &ModelFile) -> String {
    format!(
        "[[models]]\nid = \"{id}\"\nname = \"{id}\"\nkind = \"{kind}\"\nengine = \"{engine}\"\n\
         license = \"MIT\"\nlanguages = [\"*\"]\nram_mb = 1\n\
         [[models.files]]\nname = \"{}\"\nurl = \"{}\"\nsize = {}\nsha256 = \"{}\"\n",
        file.name, file.url, file.size, file.sha256
    )
}

/// Lite: VAD + small ASR. Balanced and High add their own ASR and LLM.
fn catalog_on(server: &TestServer) -> Catalog {
    let vad = server.file("/vad.onnx", &payload(1_000));
    let small = server.file("/small.bin", &payload(5_000));
    let big = server.file("/big.bin", &payload(9_000));
    let llm = server.file("/llm.gguf", &payload(7_000));
    let tier = |name: &str, asr: &str, llm: Option<&str>| {
        format!(
            "[tiers.{name}]\ndenoise = false\nvad = true\nllm_cleanup = {}\nmeeting_refine = false\n\
             diarization = false\nvad_model = \"vad\"\nasr_model = \"{asr}\"\n{}\n",
            llm.is_some(),
            llm.map(|id| format!("llm_model = \"{id}\"\n"))
                .unwrap_or_default()
        )
    };
    let toml = [
        "schema = 1\nversion = \"test\"\n".to_owned(),
        tier("lite", "small", None),
        tier("balanced", "big", Some("llm")),
        tier("high", "big", Some("llm")),
        entry("vad", "vad", "sherpa-onnx", &vad),
        entry("small", "asr", "whisper.cpp", &small),
        entry("big", "asr", "whisper.cpp", &big),
        entry("llm", "llm", "llama.cpp", &llm),
    ]
    .concat();
    Catalog::from_toml(&toml).unwrap()
}

fn eight_gb_laptop() -> HardwareInfo {
    HardwareInfo {
        total_ram_bytes: 8 << 30,
        free_ram_bytes: 3 << 30,
        cpu_cores: 4,
        cpu: CpuFeatures {
            arch: CpuArch::X86_64,
            avx2: true,
            fma: true,
            avx512: false,
            neon: false,
        },
        gpus: vec![],
    }
}

#[test]
fn an_8_gb_machine_recommends_lite_and_downloads_only_lites_models() {
    let server = TestServer::start();
    let catalog = catalog_on(&server);
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path());

    let tier = eight_gb_laptop().recommended_tier();
    assert_eq!(tier, Tier::Lite);

    let mut last = Progress::default();
    store
        .install_tier(&catalog, tier, &AtomicBool::new(false), &mut |p| last = p)
        .unwrap();

    let mut fetched: Vec<_> = server.requests().into_iter().map(|r| r.path).collect();
    fetched.sort();
    assert_eq!(fetched, ["/small.bin", "/vad.onnx"]);
    assert_eq!(
        last,
        Progress {
            downloaded: 6_000,
            total: 6_000
        }
    );
    for model in catalog.models_for_tier(Tier::Lite) {
        assert_eq!(store.status(model), ModelStatus::Installed);
    }
    assert_eq!(
        store.status(catalog.model("llm").unwrap()),
        ModelStatus::NotInstalled
    );
}

#[test]
fn a_tier_that_diarizes_also_downloads_its_speaker_models() {
    let server = TestServer::start();
    let vad = server.file("/vad.onnx", &payload(1_000));
    let asr = server.file("/asr.bin", &payload(2_000));
    let segmentation = server.file("/segmentation.onnx", &payload(300));
    let embedding = server.file("/embedding.onnx", &payload(400));
    let tier = |name: &str| {
        format!(
            "[tiers.{name}]\ndenoise = false\nvad = true\nllm_cleanup = false\n\
             meeting_refine = true\ndiarization = true\nvad_model = \"vad\"\nasr_model = \"asr\"\n\
             segmentation_model = \"segmentation\"\nspeaker_model = \"embedding\"\n"
        )
    };
    let speaker = |id: &str, file: &ModelFile| {
        entry(id, id, "sherpa-onnx", file).replace("[[models", "[[speaker_models")
    };
    let toml = [
        "schema = 1\nversion = \"test\"\n".to_owned(),
        tier("lite"),
        tier("balanced"),
        tier("high"),
        entry("vad", "vad", "sherpa-onnx", &vad),
        entry("asr", "asr", "whisper.cpp", &asr),
        speaker("segmentation", &segmentation),
        speaker("embedding", &embedding),
    ]
    .concat();
    let catalog = Catalog::from_toml(&toml).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path());

    let mut last = Progress::default();
    store
        .install_tier(&catalog, Tier::Balanced, &AtomicBool::new(false), &mut |p| {
            last = p
        })
        .unwrap();

    assert_eq!(
        last,
        Progress {
            downloaded: 3_700,
            total: 3_700
        }
    );
    for model in catalog.speaker_models_for_tier(Tier::Balanced) {
        assert_eq!(store.status(model), ModelStatus::Installed);
    }
}

#[test]
fn installing_a_tier_again_fetches_nothing_and_upgrading_fetches_only_the_difference() {
    let server = TestServer::start();
    let catalog = catalog_on(&server);
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path());
    let cancel = AtomicBool::new(false);

    store
        .install_tier(&catalog, Tier::Lite, &cancel, &mut |_| {})
        .unwrap();
    store
        .install_tier(&catalog, Tier::Lite, &cancel, &mut |_| {})
        .unwrap();
    assert_eq!(server.requests().len(), 2);

    store
        .install_tier(&catalog, Tier::Balanced, &cancel, &mut |_| {})
        .unwrap();
    let mut fetched: Vec<_> = server.requests().into_iter().map(|r| r.path).collect();
    fetched.sort();
    assert_eq!(
        fetched,
        ["/big.bin", "/llm.gguf", "/small.bin", "/vad.onnx"]
    );
}
