//! BirdNET spike, option 1: run BirdNET .tflite models from Rust through the
//! TensorFlow Lite C library, loaded at run time.
//!
//!   tflite-try <tensorflowlite_c.dll> <data_dir> <model.tflite> <model_key>
//!
//! Runs every saved segment of every clip in manifest.json (single segments,
//! one thread, like the Python reference), reports the largest difference from
//! the reference logits, then times a batch of 8 segments.

use libloading::{Library, Symbol};
use std::ffi::{c_char, c_int, c_void, CString};
use std::path::Path;
use std::time::Instant;

const SEG_N: usize = 144_000;

#[repr(C)]
struct Opaque {
    _p: [u8; 0],
}
type Model = Opaque;
type Options = Opaque;
type Interp = Opaque;
type Tensor = Opaque;
type Status = c_int; // kTfLiteOk = 0

/// The few C API functions the engine needs.
struct TfLite<'l> {
    model_create: Symbol<'l, unsafe extern "C" fn(*const c_char) -> *mut Model>,
    options_create: Symbol<'l, unsafe extern "C" fn() -> *mut Options>,
    options_threads: Symbol<'l, unsafe extern "C" fn(*mut Options, i32)>,
    interp_create: Symbol<'l, unsafe extern "C" fn(*const Model, *const Options) -> *mut Interp>,
    interp_delete: Symbol<'l, unsafe extern "C" fn(*mut Interp)>,
    resize_input: Symbol<'l, unsafe extern "C" fn(*mut Interp, i32, *const c_int, i32) -> Status>,
    allocate: Symbol<'l, unsafe extern "C" fn(*mut Interp) -> Status>,
    input: Symbol<'l, unsafe extern "C" fn(*const Interp, i32) -> *mut Tensor>,
    output: Symbol<'l, unsafe extern "C" fn(*const Interp, i32) -> *const Tensor>,
    invoke: Symbol<'l, unsafe extern "C" fn(*mut Interp) -> Status>,
    copy_from: Symbol<'l, unsafe extern "C" fn(*mut Tensor, *const c_void, usize) -> Status>,
    copy_to: Symbol<'l, unsafe extern "C" fn(*const Tensor, *mut c_void, usize) -> Status>,
    byte_size: Symbol<'l, unsafe extern "C" fn(*const Tensor) -> usize>,
    version: Symbol<'l, unsafe extern "C" fn() -> *const c_char>,
}

impl<'l> TfLite<'l> {
    fn load(lib: &'l Library) -> Result<Self, libloading::Error> {
        unsafe {
            Ok(Self {
                model_create: lib.get(b"TfLiteModelCreateFromFile")?,
                options_create: lib.get(b"TfLiteInterpreterOptionsCreate")?,
                options_threads: lib.get(b"TfLiteInterpreterOptionsSetNumThreads")?,
                interp_create: lib.get(b"TfLiteInterpreterCreate")?,
                interp_delete: lib.get(b"TfLiteInterpreterDelete")?,
                resize_input: lib.get(b"TfLiteInterpreterResizeInputTensor")?,
                allocate: lib.get(b"TfLiteInterpreterAllocateTensors")?,
                input: lib.get(b"TfLiteInterpreterGetInputTensor")?,
                output: lib.get(b"TfLiteInterpreterGetOutputTensor")?,
                invoke: lib.get(b"TfLiteInterpreterInvoke")?,
                copy_from: lib.get(b"TfLiteTensorCopyFromBuffer")?,
                copy_to: lib.get(b"TfLiteTensorCopyToBuffer")?,
                byte_size: lib.get(b"TfLiteTensorByteSize")?,
                version: lib.get(b"TfLiteVersion")?,
            })
        }
    }
}

/// One interpreter for one model, fed `batch` segments at a time.
struct Runner<'a, 'l> {
    tf: &'a TfLite<'l>,
    interp: *mut Interp,
    batch: usize,
}

impl<'a, 'l> Runner<'a, 'l> {
    fn new(tf: &'a TfLite<'l>, model_path: &str, threads: i32) -> Self {
        let c = CString::new(model_path).unwrap();
        unsafe {
            let model = (tf.model_create)(c.as_ptr());
            assert!(!model.is_null(), "model could not be loaded: {model_path}");
            let opts = (tf.options_create)();
            (tf.options_threads)(opts, threads);
            let interp = (tf.interp_create)(model, opts);
            assert!(!interp.is_null(), "interpreter could not be created");
            let mut r = Self { tf, interp, batch: 0 };
            r.resize(1);
            r
        }
    }

    fn resize(&mut self, batch: usize) {
        if self.batch == batch {
            return;
        }
        let dims = [batch as c_int, SEG_N as c_int];
        unsafe {
            assert_eq!((self.tf.resize_input)(self.interp, 0, dims.as_ptr(), 2), 0, "resize failed");
            assert_eq!((self.tf.allocate)(self.interp), 0, "allocate failed");
        }
        self.batch = batch;
    }

    /// Logits for `segments` (batch x 144000), batch x n_species.
    fn run(&mut self, segments: &[f32]) -> Vec<f32> {
        let batch = segments.len() / SEG_N;
        self.resize(batch);
        unsafe {
            let input = (self.tf.input)(self.interp, 0);
            assert_eq!((self.tf.copy_from)(input, segments.as_ptr().cast(), segments.len() * 4), 0);
            assert_eq!((self.tf.invoke)(self.interp), 0, "invoke failed");
            let out = (self.tf.output)(self.interp, 0);
            let n = (self.tf.byte_size)(out) / 4;
            let mut v = vec![0f32; n];
            assert_eq!((self.tf.copy_to)(out, v.as_mut_ptr().cast(), n * 4), 0);
            v
        }
    }
}

impl Drop for Runner<'_, '_> {
    fn drop(&mut self) {
        unsafe { (self.tf.interp_delete)(self.interp) }
    }
}

fn read_f32(path: &Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dll, data, model_path, key) = (&args[0], Path::new(&args[1]), &args[2], &args[3]);
    let lib = unsafe { Library::new(dll) }.expect("load tensorflowlite_c");
    let tf = TfLite::load(&lib).expect("C API symbols");
    let version = unsafe { std::ffi::CStr::from_ptr((tf.version)()) }.to_string_lossy().into_owned();
    println!("TFLite {version}");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(data.join("manifest.json")).unwrap()).unwrap();

    let t0 = Instant::now();
    let mut runner = Runner::new(&tf, model_path, 1);
    println!("{key}: loaded in {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);

    let mut worst = 0f32;
    let mut all_segments = Vec::new();
    for (clip, info) in manifest["clips"].as_object().unwrap() {
        let n = info["segments"].as_u64().unwrap() as usize;
        let segs = read_f32(&data.join(format!("{clip}.segments.f32")));
        let want = read_f32(&data.join(format!("{clip}.{key}.logits.f32")));
        let n_species = want.len() / n;
        let t = Instant::now();
        let mut clip_worst = 0f32;
        for s in 0..n {
            let got = runner.run(&segs[s * SEG_N..(s + 1) * SEG_N]);
            assert_eq!(got.len(), n_species);
            for (g, w) in got.iter().zip(&want[s * n_species..(s + 1) * n_species]) {
                clip_worst = clip_worst.max((g - w).abs());
            }
        }
        let per = t.elapsed().as_secs_f64() / n as f64;
        let py = manifest["models"][key.as_str()]["per_segment_s"][clip.as_str()].as_f64().unwrap_or(f64::NAN);
        println!(
            "{key} {clip}: max |logit diff| {clip_worst:.3e}; {:.1} ms/segment (Python {:.1})",
            per * 1000.0,
            py * 1000.0
        );
        worst = worst.max(clip_worst);
        all_segments.extend_from_slice(&segs);
    }
    println!("{key}: worst |logit diff| {worst:.3e}");

    // batch of 8, one thread, and 8 segments on 4 threads
    let batch = &all_segments[..8 * SEG_N];
    runner.run(batch);
    let t = Instant::now();
    runner.run(batch);
    println!("{key}: batch of 8, 1 thread: {:.1} ms/segment", t.elapsed().as_secs_f64() * 1000.0 / 8.0);
    drop(runner);
    let mut r4 = Runner::new(&tf, model_path, 4);
    r4.run(batch);
    let t = Instant::now();
    r4.run(batch);
    println!("{key}: batch of 8, 4 threads: {:.1} ms/segment", t.elapsed().as_secs_f64() * 1000.0 / 8.0);
}
