//! The bundled, versioned model manifest and its tier presets (spec §6).

use wf_models::{Catalog, ModelKind, Tier};

#[test]
fn lite_preset_is_rules_only_with_silero_and_a_small_asr_model() {
    let catalog = Catalog::bundled();
    let lite = catalog.preset(Tier::Lite);
    assert!(lite.vad && !lite.llm_cleanup && !lite.denoise && !lite.meeting_refine);

    let kinds: Vec<ModelKind> = catalog
        .models_for_tier(Tier::Lite)
        .iter()
        .map(|m| m.kind)
        .collect();
    assert_eq!(kinds, [ModelKind::Vad, ModelKind::Asr]);
    let total: u64 = catalog
        .models_for_tier(Tier::Lite)
        .iter()
        .map(|m| m.download_size())
        .sum();
    assert!(total < 500_000_000, "Lite downloads {total} bytes");
}

const SHA: &str = "6b99cbfd39246b6706f98ec13c7c50c6b299181f2474fa05cbc8046acc274396";

/// A minimal valid manifest; tests break one thing at a time.
fn manifest(lite_asr: &str, file_name: &str, url: &str, sha256: &str) -> String {
    let tier = |name: &str| {
        format!(
            "[tiers.{name}]\ndenoise = false\nvad = true\nllm_cleanup = false\n\
             meeting_refine = false\ndiarization = false\nvad_model = \"vad\"\nasr_model = \"{}\"\n",
            if name == "lite" { lite_asr } else { "asr" }
        )
    };
    format!(
        "schema = 1\nversion = \"test\"\n{}{}{}\n\
         [[models]]\nid = \"vad\"\nname = \"VAD\"\nkind = \"vad\"\nengine = \"sherpa-onnx\"\n\
         license = \"MIT\"\nlanguages = [\"*\"]\nram_mb = 1\n\
         [[models.files]]\nname = \"vad.onnx\"\nurl = \"https://example.org/vad.onnx\"\nsize = 1\nsha256 = \"{SHA}\"\n\
         [[models]]\nid = \"asr\"\nname = \"ASR\"\nkind = \"asr\"\nengine = \"whisper.cpp\"\n\
         license = \"MIT\"\nlanguages = [\"*\"]\nram_mb = 1\n\
         [[models.files]]\nname = \"{file_name}\"\nurl = \"{url}\"\nsize = 1\nsha256 = \"{sha256}\"\n",
        tier("lite"),
        tier("balanced"),
        tier("high"),
    )
}

fn valid() -> String {
    manifest("asr", "model.bin", "https://example.org/model.bin", SHA)
}

#[test]
fn a_valid_manifest_parses() {
    let catalog = Catalog::from_toml(&valid()).unwrap();
    assert_eq!(catalog.version, "test");
    assert_eq!(catalog.models_for_tier(Tier::High).len(), 2);
}

#[test]
fn a_tier_naming_a_missing_model_is_rejected() {
    let err =
        Catalog::from_toml(&manifest("nope", "m.bin", "https://example.org/m", SHA)).unwrap_err();
    assert!(err.to_string().contains("nope"), "{err}");
}

#[test]
fn a_tier_using_a_model_for_the_wrong_stage_is_rejected() {
    // Lite's ASR slot names the VAD model.
    assert!(Catalog::from_toml(&manifest("vad", "m.bin", "https://example.org/m", SHA)).is_err());
}

#[test]
fn checksums_must_be_64_hex_digits() {
    let short = &SHA[..63];
    assert!(Catalog::from_toml(&manifest("asr", "m.bin", "https://example.org/m", short)).is_err());
    let upper = SHA.to_uppercase();
    assert!(
        Catalog::from_toml(&manifest("asr", "m.bin", "https://example.org/m", &upper)).is_err()
    );
}

#[test]
fn downloads_must_use_https_except_from_loopback() {
    // ADR-0004: HTTPS only. Loopback is allowed so tests can serve files locally.
    assert!(Catalog::from_toml(&manifest("asr", "m.bin", "http://example.org/m", SHA)).is_err());
    assert!(Catalog::from_toml(&manifest("asr", "m.bin", "http://127.0.0.1:8080/m", SHA)).is_ok());
}

#[test]
fn file_names_cannot_escape_the_model_folder() {
    for name in [
        "../evil.bin",
        "/etc/passwd",
        "a/b.bin",
        "a\\\\b.bin",
        "",
        "..",
    ] {
        let toml = manifest("asr", name, "https://example.org/m", SHA);
        assert!(Catalog::from_toml(&toml).is_err(), "{name:?} accepted");
    }
}

#[test]
fn an_unknown_schema_is_rejected() {
    let future = valid().replacen("schema = 1", "schema = 2", 1);
    assert!(Catalog::from_toml(&future).is_err());
}

#[test]
fn llm_cleanup_needs_an_llm_model() {
    let toml = valid().replacen("llm_cleanup = false", "llm_cleanup = true", 1);
    assert!(Catalog::from_toml(&toml).is_err());
}

#[test]
fn balanced_and_high_run_llm_cleanup_with_more_memory_than_lite() {
    let catalog = Catalog::bundled();
    let ram = |tier| {
        catalog
            .models_for_tier(tier)
            .iter()
            .map(|m| m.ram_mb)
            .sum::<u64>()
    };
    for tier in [Tier::Balanced, Tier::High] {
        assert!(catalog.preset(tier).llm_cleanup, "{tier:?}");
        assert!(
            catalog
                .models_for_tier(tier)
                .iter()
                .any(|m| m.kind == ModelKind::Llm)
        );
    }
    assert!(ram(Tier::Lite) < ram(Tier::Balanced) && ram(Tier::Balanced) < ram(Tier::High));
}

#[test]
fn a_model_stops_being_a_candidate_when_a_tier_picks_it() {
    let catalog = Catalog::bundled();
    assert_eq!(catalog.schema, Catalog::SCHEMA);
    let picked: Vec<&str> = [Tier::Lite, Tier::Balanced, Tier::High]
        .into_iter()
        .flat_map(|tier| {
            let models = catalog.models_for_tier(tier);
            models
                .into_iter()
                .chain(catalog.speaker_models_for_tier(tier))
        })
        .map(|model| model.id.as_str())
        .collect();
    for model in &catalog.models {
        assert!(model.download_size() > 0, "{}", model.id);
        assert_eq!(
            model.candidate,
            !picked.contains(&model.id.as_str()),
            "{}",
            model.id
        );
    }
}

/// `docs/benchmarks.md` has the reasons. If a default moves, move the numbers with it.
#[test]
fn the_tiers_use_the_benchmarked_defaults() {
    let catalog = Catalog::bundled();
    let picks = |tier| {
        let preset = catalog.preset(tier);
        (preset.asr_model.as_str(), preset.llm_model.as_deref())
    };
    assert_eq!(picks(Tier::Lite), ("whisper-small-q8_0", None));
    assert_eq!(
        picks(Tier::Balanced),
        ("whisper-large-v3-turbo-q5_0", Some("qwen3-1.7b-q4_k_m"))
    );
    assert_eq!(
        picks(Tier::High),
        (
            "whisper-large-v3-turbo",
            Some("qwen3-4b-instruct-2507-q4_k_m")
        )
    );
}

#[test]
fn balanced_and_high_diarize_with_a_segmentation_and_an_embedding_model() {
    let catalog = Catalog::bundled();
    for tier in [Tier::Balanced, Tier::High] {
        let preset = catalog.preset(tier);
        assert!(preset.meeting_refine && preset.diarization, "{tier:?}");
        let kinds: Vec<ModelKind> = catalog
            .speaker_models_for_tier(tier)
            .iter()
            .map(|m| m.kind)
            .collect();
        assert_eq!(kinds, [ModelKind::Segmentation, ModelKind::Embedding]);
    }
}

#[test]
fn lite_downloads_no_speaker_models() {
    let catalog = Catalog::bundled();
    assert!(!catalog.preset(Tier::Lite).diarization);
    assert!(catalog.speaker_models_for_tier(Tier::Lite).is_empty());
}

#[test]
fn every_speaker_model_has_an_open_license_and_a_pinned_checksum() {
    let catalog = Catalog::bundled();
    let speaker_kinds = [ModelKind::Segmentation, ModelKind::Embedding];
    let speaker_models: Vec<_> = catalog
        .models
        .iter()
        .filter(|model| speaker_kinds.contains(&model.kind))
        .collect();
    assert!(!speaker_models.is_empty());
    for model in speaker_models {
        assert!(
            ["MIT", "Apache-2.0"].contains(&model.license.as_str()),
            "{}: {}",
            model.id,
            model.license
        );
        assert!(model.download_size() > 0, "{}", model.id);
    }
}

/// [`manifest`] plus a speaker model on Lite.
fn with_speaker_slot(segmentation: &str) -> String {
    let speaker = |id: &str, kind: &str| {
        format!(
            "[[models]]\nid = \"{id}\"\nname = \"{id}\"\nkind = \"{kind}\"\n\
             engine = \"sherpa-onnx\"\nlicense = \"MIT\"\nlanguages = [\"*\"]\nram_mb = 1\n\
             [[models.files]]\nname = \"m.onnx\"\nurl = \"https://example.org/m\"\n\
             size = 1\nsha256 = \"{SHA}\"\n"
        )
    };
    let slots = format!(
        "diarization = true\nsegmentation_model = \"{segmentation}\"\n\
         speaker_model = \"embedding\"\n"
    );
    format!(
        "{}{}{}",
        valid().replacen("diarization = false\n", &slots, 1),
        speaker("segmentation", "segmentation"),
        speaker("embedding", "embedding"),
    )
}

#[test]
fn diarization_slots_parse_when_they_name_the_right_kinds() {
    let catalog = Catalog::from_toml(&with_speaker_slot("segmentation")).unwrap();
    assert_eq!(catalog.speaker_models_for_tier(Tier::Lite).len(), 2);
}

#[test]
fn a_diarization_slot_must_name_a_model_of_its_kind() {
    assert!(Catalog::from_toml(&with_speaker_slot("embedding")).is_err());
    assert!(Catalog::from_toml(&with_speaker_slot("nope")).is_err());
}

#[test]
fn diarization_needs_both_speaker_models() {
    let toml = with_speaker_slot("segmentation").replacen("speaker_model = \"embedding\"\n", "", 1);
    assert!(Catalog::from_toml(&toml).is_err());
}
