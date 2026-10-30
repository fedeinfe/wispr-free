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
fn bundled_models_stay_candidates_until_the_benchmark_picks_defaults() {
    let catalog = Catalog::bundled();
    assert_eq!(catalog.schema, Catalog::SCHEMA);
    assert!(
        catalog
            .models
            .iter()
            .all(|m| m.candidate && m.download_size() > 0)
    );
}
