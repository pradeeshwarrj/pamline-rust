//! BirdNET spike, option 3: can tract (pure Rust) run BirdNET .tflite models?
//!
//!   tract-try <data_dir> <model.tflite> <model_key>
//!
//! Loads the model, runs every saved segment of every clip in manifest.json,
//! and reports the largest difference from the Python reference logits.

use std::path::Path;
use std::time::Instant;
use tract_tflite::prelude::*;

const SEG_N: usize = 144_000;

fn read_f32(path: &Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn main() -> TractResult<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (data, model_path, key) = (Path::new(&args[0]), &args[1], &args[2]);
    let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(data.join("manifest.json"))?)?;

    eprintln!("loading {model_path}");
    let t0 = Instant::now();
    let model = match tract_tflite::tflite().model_for_path(model_path) {
        Ok(m) => m,
        Err(e) => {
            println!("LOAD FAILED: {e:?}");
            return Ok(());
        }
    };
    eprintln!("loaded in {:?}; nodes: {}", t0.elapsed(), model.nodes().len());
    let model = match model.with_input_fact(0, f32::fact([1, SEG_N]).into()).and_then(|m| m.into_optimized()) {
        Ok(m) => m,
        Err(e) => {
            println!("OPTIMIZE FAILED: {e:?}");
            return Ok(());
        }
    };
    let runnable = model.into_runnable()?;

    let mut worst = 0f32;
    for (clip, info) in manifest["clips"].as_object().unwrap() {
        let n = info["segments"].as_u64().unwrap() as usize;
        let segs = read_f32(&data.join(format!("{clip}.segments.f32")));
        let want = read_f32(&data.join(format!("{clip}.{key}.logits.f32")));
        let n_species = want.len() / n;
        let t = Instant::now();
        let mut clip_worst = 0f32;
        for s in 0..n {
            let input = tract_ndarray::Array2::from_shape_vec((1, SEG_N), segs[s * SEG_N..(s + 1) * SEG_N].to_vec())?;
            let out = match runnable.run(tvec!(input.into_tensor().into())) {
                Ok(o) => o,
                Err(e) => {
                    println!("RUN FAILED on {clip} segment {s}: {e:?}");
                    return Ok(());
                }
            };
            let view = out[0].to_plain_array_view::<f32>()?;
            let got: Vec<f32> = view.iter().copied().collect();
            for (g, w) in got.iter().zip(&want[s * n_species..(s + 1) * n_species]) {
                clip_worst = clip_worst.max((g - w).abs());
            }
        }
        let per = t.elapsed().as_secs_f64() / n as f64;
        println!("{key} {clip}: max |logit diff| {clip_worst:.3e}, {:.1} ms/segment", per * 1000.0);
        worst = worst.max(clip_worst);
    }
    println!("{key}: worst |logit diff| {worst:.3e}");
    Ok(())
}
