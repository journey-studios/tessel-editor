//! P3 (BL082.1/BL082.2/BL036): the pinned pilot FILM-GRAIN and KEYS corpus, run against the Rust core.
//!
//! `docs/migration/fixtures/pilot-fx/grain-cases.json` was derived from the pilot's `filmGrain`
//! shader, its integer hashes and its keys resolver by `scripts/pilot-grain-corpus.py` (standalone,
//! shares no routine with the Rust reference or with the runtime's GLSL; `grain-manifest.json` names
//! the origin snapshot, the blob ids and sha256 of every source, the documented differences and the
//! blockers). Agreeing here is two derivations of the origin's formulas agreeing, never the
//! reference compared with itself. The integers (hashes, cells, float32 bit patterns) are compared
//! exactly; only the final bytes carry a tolerance.
//!
//! What it does NOT prove: the origin's golden `filmGrain.png` (it needs the origin's harness, which
//! is not run here: manifest blocker `ORIGIN_GOLDEN_NOT_COMPARABLE`) and the origin's own outputs
//! (blocker `ORIGIN_NOT_EXECUTED`). The corpus hash and the origin blobs are checked by
//! `scripts/pilot-grain-corpus.py verify [--origin PATH]`. The same vectors are compared with the
//! GLSL in a browser (`runtime/test/directed-scene-grain-browser.test.mjs`).

use core::prelude::v1::test;
use serde_json::Value;
use tessel_editor_core::directed_scene::fx::{
    self, FxEasing, FxKey, FxPass, FxValue, Reseed, apply_pass_to_srgb, colored_triangular,
    fnv1a32, grain_cell_px, grain_encoded_delta, hash_pixel, lowbias32, resolve_keys, seed_for,
    triangular,
};

const CORPUS: &str = include_str!("../../../docs/migration/fixtures/pilot-fx/grain-cases.json");
const MANIFEST: &str =
    include_str!("../../../docs/migration/fixtures/pilot-fx/grain-manifest.json");

fn corpus() -> Value {
    serde_json::from_str(CORPUS).unwrap()
}

fn u32_of(value: &Value) -> u32 {
    u32::try_from(value.as_u64().unwrap()).unwrap()
}

#[test]
fn the_manifest_names_the_grain_sources_the_differences_and_the_blockers_it_could_not_close() {
    let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
    assert_eq!(manifest["origin"]["snapshot"], "d40ac3f7c0");
    assert_eq!(manifest["corpus"]["file"], "grain-cases.json");
    assert_eq!(manifest["corpus"]["sha256"].as_str().unwrap().len(), 64);
    let sources = manifest["sources"].as_array().unwrap();
    for source in sources {
        assert_eq!(source["git_blob"].as_str().unwrap().len(), 40, "{source}");
        assert_eq!(source["sha256"].as_str().unwrap().len(), 64, "{source}");
    }
    let paths: Vec<_> = sources
        .iter()
        .map(|s| s["path"].as_str().unwrap())
        .collect();
    for needed in [
        "packages/scene-engine/src/fx/catalog/filmGrain.ts",
        "packages/scene-engine/src/runtime/fx/shaders/filmGrain.ts",
        "packages/scene-engine/src/runtime/fx/shaders/chunks/hash.ts",
        "packages/scene-engine/src/fx/fx-hash.ts",
        "packages/scene-engine/src/keyframes/resolve-keyframes.ts",
        "packages/scene-engine/src/fx/catalog/__golden__/filmGrain.png",
    ] {
        assert!(paths.contains(&needed), "{needed}");
    }
    let ids = |key: &str| -> Vec<String> {
        manifest[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap().to_string())
            .collect()
    };
    let blockers = ids("blockers");
    assert!(blockers.contains(&"ORIGIN_GOLDEN_NOT_COMPARABLE".to_string()));
    assert!(blockers.contains(&"ORIGIN_NOT_EXECUTED".to_string()));
    let differences = ids("documented_differences");
    for needed in [
        "ENCODED_SOURCE",
        "ALPHA_PRESERVED",
        "RESIZE_REFUSED_NOT_DROPPED",
    ] {
        assert!(differences.contains(&needed.to_string()), "{needed}");
    }
    // The counts the manifest records are the corpus's own: a corpus cut down is noticed.
    let corpus = corpus();
    assert_eq!(
        manifest["corpus"]["counts"]["hash_pixel"].as_u64().unwrap() as usize,
        corpus["hashes"]["hash_pixel"].as_array().unwrap().len()
    );
    assert_eq!(
        manifest["corpus"]["counts"]["pixel_cases"]
            .as_u64()
            .unwrap() as usize,
        corpus["pixels"].as_array().unwrap().len()
    );
}

#[test]
fn the_integer_hashes_are_the_pilots_to_the_last_bit() {
    let hashes = &corpus()["hashes"];
    let mut compared = 0;
    for case in hashes["fnv1a32"].as_array().unwrap() {
        let text = case["text"].as_str().unwrap();
        assert_eq!(fnv1a32(text), u32_of(&case["hash"]), "fnv1a32({text:?})");
        compared += 1;
    }
    for case in hashes["lowbias32"].as_array().unwrap() {
        assert_eq!(
            lowbias32(u32_of(&case["in"])),
            u32_of(&case["out"]),
            "{case}"
        );
        compared += 1;
    }
    for case in hashes["seed_for"].as_array().unwrap() {
        let reseed = match case["reseed"].as_str().unwrap() {
            "per-frame" => Reseed::PerFrame,
            _ => Reseed::Static,
        };
        let frame = case["frame"].as_i64().unwrap();
        assert_eq!(
            seed_for(reseed, u32_of(&case["base"]), frame),
            Some(u32_of(&case["seed"])),
            "{case}"
        );
        compared += 1;
    }
    for case in hashes["hash_pixel"].as_array().unwrap() {
        assert_eq!(
            hash_pixel(
                u32_of(&case["seed"]),
                u32_of(&case["x"]),
                u32_of(&case["y"])
            ),
            u32_of(&case["hash"]),
            "{case}"
        );
        compared += 1;
    }
    for case in hashes["triangular"].as_array().unwrap() {
        let value = triangular(u32_of(&case["hash"]));
        assert_eq!(value.to_bits(), u32_of(&case["bits"]), "{case}");
        assert_eq!(f64::from(value), case["value"].as_f64().unwrap(), "{case}");
        compared += 1;
    }
    for case in hashes["colored"].as_array().unwrap() {
        let channels = colored_triangular(u32_of(&case["hash"]));
        for (channel, bits) in channels.iter().zip(case["bits"].as_array().unwrap()) {
            assert_eq!(channel.to_bits(), u32_of(bits), "{case}");
        }
        compared += 1;
    }
    assert!(compared > 150, "compared {compared}");
}

fn typed(param: &str, raw: &Value) -> FxValue {
    let spec = fx::effect("filmGrain")
        .into_iter()
        .chain(fx::effect("vignette"))
        .flat_map(|effect| effect.params.iter())
        .find(|spec| spec.id == param)
        .unwrap();
    fx::parse_param(spec, raw).unwrap()
}

fn same(left: FxValue, right: FxValue, exact: bool) -> bool {
    let close = |a: f64, b: f64| {
        if exact {
            a.to_bits() == b.to_bits()
        } else {
            (a - b).abs() <= 1e-12
        }
    };
    match (left, right) {
        (FxValue::Number(a), FxValue::Number(b)) => close(a, b),
        (FxValue::Bool(a), FxValue::Bool(b)) => a == b,
        (
            FxValue::Color { r, g, b },
            FxValue::Color {
                r: r2,
                g: g2,
                b: b2,
            },
        ) => close(r, r2) && close(g, g2) && close(b, b2),
        (FxValue::Point { x, y }, FxValue::Point { x: x2, y: y2 }) => close(x, x2) && close(y, y2),
        _ => false,
    }
}

#[test]
fn the_keys_resolver_is_the_pilots_on_every_kind_of_value() {
    let mut compared = 0;
    let mut eased = 0;
    for curve in corpus()["keys"].as_array().unwrap() {
        let param = curve["param"].as_str().unwrap();
        let keys: Vec<FxKey> = curve["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| FxKey {
                frame: key["frame"].as_i64().unwrap(),
                value: typed(param, &key["value"]),
                easing: key
                    .get("easing")
                    .and_then(Value::as_str)
                    .map_or(FxEasing::Linear, |name| FxEasing::parse(name).unwrap()),
            })
            .collect();
        for query in curve["queries"].as_array().unwrap() {
            let frame = query["frame"].as_i64().unwrap();
            let want = typed(param, &query["value"]);
            let exact = query["exact"].as_bool().unwrap();
            let got = resolve_keys(&keys, frame).unwrap();
            assert!(
                same(got, want, exact),
                "{} at {frame}: {got:?} against {want:?}",
                curve["id"]
            );
            compared += 1;
            eased += usize::from(!exact);
        }
    }
    assert!(compared > 100, "compared {compared}");
    assert!(eased > 20, "the corpus holds {eased} interpolated queries");
}

fn grain_pass(case: &Value) -> FxPass {
    let params = &case["params"];
    FxPass {
        track_id: "t".into(),
        clip_id: "g".into(),
        effect: "filmGrain".into(),
        frame_in_clip: 0,
        weight: 1.0,
        amount: case["pass_amount"].as_f64().unwrap(),
        params: vec![
            (
                "amount".into(),
                FxValue::Number(params["amount"].as_f64().unwrap()),
            ),
            (
                "size".into(),
                FxValue::Number(params["size"].as_f64().unwrap()),
            ),
            (
                "luma".into(),
                FxValue::Number(params["luma"].as_f64().unwrap()),
            ),
            (
                "colored".into(),
                FxValue::Bool(params["colored"].as_bool().unwrap()),
            ),
            ("intensity".into(), FxValue::Number(1.0)),
        ],
        seed: Some(u32_of(&case["seed"])),
    }
}

#[test]
fn the_grain_pixels_are_the_pilots_cell_hash_noise_and_bytes() {
    let mut probes = 0;
    let mut grained = 0;
    for case in corpus()["pixels"].as_array().unwrap() {
        let pass = grain_pass(case);
        let height = case["height"].as_u64().unwrap() as usize;
        let width = case["width"].as_u64().unwrap() as usize;
        let source = case["source_rgb"].as_array().unwrap();
        let rgb: Vec<f64> = source.iter().map(|c| c.as_f64().unwrap() / 255.0).collect();
        for probe in case["probes"].as_array().unwrap() {
            let (x, y) = (
                probe["x"].as_u64().unwrap() as usize,
                probe["y"].as_u64().unwrap() as usize,
            );
            let out = apply_pass_to_srgb(&pass, [rgb[0], rgb[1], rgb[2]], x, y, width, height)
                .unwrap_or_else(|| panic!("{} {x},{y}", case["id"]));
            if let Some(hash) = probe.get("hash") {
                // The integers first, exactly: the cell, its hash, the float32 bits of the noise.
                let cell_size = grain_cell_px(case["params"]["size"].as_f64().unwrap(), height);
                let gl_y = (height - 1 - y) as f32 + 0.5;
                let cell = [
                    ((x as f32 + 0.5) / cell_size as f32).floor() as u64,
                    (gl_y / cell_size as f32).floor() as u64,
                ];
                assert_eq!(
                    cell,
                    [
                        probe["cell"][0].as_u64().unwrap(),
                        probe["cell"][1].as_u64().unwrap()
                    ],
                    "{} {x},{y}: cell",
                    case["id"]
                );
                assert_eq!(
                    hash_pixel(pass.seed.unwrap(), cell[0] as u32, cell[1] as u32),
                    u32_of(hash),
                    "{} {x},{y}: hash",
                    case["id"]
                );
                let noise = grain_encoded_delta(&pass, x, y, height).unwrap();
                for (channel, bits) in noise.iter().zip(probe["noise_bits"].as_array().unwrap()) {
                    assert_eq!(
                        channel.to_bits(),
                        u32_of(bits),
                        "{} {x},{y}: noise",
                        case["id"]
                    );
                }
                grained += 1;
            }
            for channel in 0..3 {
                let want = probe["expected"][channel].as_f64().unwrap();
                let got = out[channel] * 255.0;
                assert!(
                    (got - want).abs() < 1e-6,
                    "{} {x},{y} channel {channel}: {got} against {want}",
                    case["id"]
                );
            }
            probes += 1;
        }
    }
    assert!(probes > 100, "compared {probes}");
    assert!(grained > 80, "{grained} probes carry a noise");
}

/// `(width, height, pass amount, [(x, y, pinned bits)])` of one pixel case.
type CaseProbes = (usize, usize, f64, Vec<(usize, usize, [u64; 2])>);

#[test]
fn the_corpus_discriminates_the_mistakes_a_port_would_make() {
    // Each mistake is applied to the formula and must disagree with the pinned probes somewhere,
    // or the corpus would not notice it.
    let corpus = corpus();
    let cases = corpus["pixels"].as_array().unwrap();
    let probes_of = |id: &str| -> CaseProbes {
        let case = cases.iter().find(|case| case["id"] == id).unwrap();
        (
            case["height"].as_u64().unwrap() as usize,
            case["width"].as_u64().unwrap() as usize,
            case["params"]["size"].as_f64().unwrap(),
            case["probes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|probe| {
                    (
                        probe["x"].as_u64().unwrap() as usize,
                        probe["y"].as_u64().unwrap() as usize,
                        [
                            probe["cell"][0].as_u64().unwrap(),
                            probe["cell"][1].as_u64().unwrap(),
                        ],
                    )
                })
                .collect(),
        )
    };
    let differs = |id: &str, cell: &dyn Fn(usize, usize, usize, f64) -> [u64; 2]| -> bool {
        let (height, _, size, probes) = probes_of(id);
        probes
            .iter()
            .any(|(x, y, expected)| cell(*x, *y, height, size) != *expected)
    };
    let correct = |x: usize, y: usize, height: usize, size: f64| -> [u64; 2] {
        let cell = grain_cell_px(size, height) as f32;
        [
            ((x as f32 + 0.5) / cell).floor() as u64,
            (((height - 1 - y) as f32 + 0.5) / cell).floor() as u64,
        ]
    };
    for id in [
        "grain-mono-fractional-cell",
        "grain-scale-applies-once-on-a-tall-picture",
    ] {
        assert!(!differs(id, &correct), "{id}: the right formula agrees");
        // The row counted from the top instead of the bottom.
        assert!(
            differs(id, &|x, y, height, size| {
                let cell = grain_cell_px(size, height) as f32;
                [
                    ((x as f32 + 0.5) / cell).floor() as u64,
                    ((y as f32 + 0.5) / cell).floor() as u64,
                ]
            }),
            "{id}: a missing flip is noticed"
        );
        // The pixel's corner instead of its centre: only a fractional cell has a border between a
        // pixel's corner and its centre (the 4 px cell of the tall picture cannot tell).
        if id.contains("fractional") {
            assert!(
                differs(id, &|x, y, height, size| {
                    let cell = grain_cell_px(size, height) as f32;
                    [
                        (x as f32 / cell).floor() as u64,
                        ((height - 1 - y) as f32 / cell).floor() as u64,
                    ]
                }),
                "{id}: the pixel corner is noticed"
            );
        }
        // The scale applied twice, or not at all: only a picture that is not 720 rows tall can tell
        // (at 720 the scale is 1), which is why the corpus holds the 1440-row case.
        let scales: &[f64] = if id.contains("tall") {
            &[2.0, 0.0]
        } else {
            &[]
        };
        for factor in scales.iter().copied() {
            assert!(
                differs(id, &|x, y, height, size| {
                    let cell = if factor == 0.0 {
                        size.max(1.0)
                    } else {
                        (size * (height as f64 / 720.0) * (height as f64 / 720.0)).max(1.0)
                    } as f32;
                    [
                        ((x as f32 + 0.5) / cell).floor() as u64,
                        (((height - 1 - y) as f32 + 0.5) / cell).floor() as u64,
                    ]
                }),
                "{id}: the scale mistake {factor} is noticed"
            );
        }
    }
    // A hash with a sum instead of a xor, or a missing constant, changes the pinned integers.
    let hashes = &corpus["hashes"]["hash_pixel"];
    let broken = |seed: u32, x: u32, y: u32| {
        lowbias32(
            lowbias32(seed.wrapping_add(0x9e37_79b9))
                .wrapping_add(x)
                .wrapping_add(y << 16),
        )
    };
    assert!(hashes.as_array().unwrap().iter().any(|case| {
        broken(
            u32_of(&case["seed"]),
            u32_of(&case["x"]),
            u32_of(&case["y"]),
        ) != u32_of(&case["hash"])
    }));
}
