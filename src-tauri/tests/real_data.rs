//! Read-only check against real recorder chunks (WAV) (not run by default).
//!
//!   PA_REAL_DIR=/path/to/day cargo test --release --test real_data -- --ignored --nocapture
//!   add PA_OUT_DIR=/tmp/out PA_MERGE_MATCH=260908_S072908 to also write one merged file.

use prepare_audio_lib::{merge, scan};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

#[test]
#[ignore]
fn real_folder() {
    // Several folders separated by '|', like dropping them together.
    let dirs: Vec<PathBuf> = std::env::var("PA_REAL_DIR").expect("PA_REAL_DIR not set").split('|').map(PathBuf::from).collect();
    let t = std::time::Instant::now();
    let s = scan::scan(&dirs, &scan::Options::default()).unwrap();
    println!("scan: {} files in {:.2?} -> output folder {}", s.files_seen, t.elapsed(), s.default_out_dir);
    for r in &s.recordings {
        println!(
            "#{:<2} {} {}–{} {:>8.2}s  {} part(s) conf={:?} -> {}  {:?}",
            r.id, r.date, r.start, r.end, r.duration, r.parts.len(), r.confidence, r.out_name, r.warnings
        );
        for p in &r.parts {
            println!("      {:<60} gap={:?} full={} src={:?} link={:?}", p.path, p.gap_to_prev, p.full_chunk, p.time_source, p.link_confidence);
        }
    }
    if let Ok(json) = std::env::var("PA_JSON") {
        std::fs::write(&json, serde_json::to_string_pretty(&s).unwrap()).unwrap();
        println!("wrote {json}");
    }
    println!("duplicates: {:?}", s.duplicates);
    println!("ignored: {:?}", s.ignored);

    // Calibration: continuity of every true join vs. joining onto the other transmitter's chunk.
    let all: Vec<&scan::Part> = s.recordings.iter().flat_map(|r| r.parts.iter()).collect();
    for r in &s.recordings {
        for w in r.parts.windows(2) {
            let good = scan::continuity_cost(&w[0], &w[1]).unwrap();
            let rivals: Vec<String> = all
                .iter()
                .filter(|o| o.seq == w[1].seq && o.path != w[1].path && o.path != w[0].path && (o.start_secs - w[1].start_secs).abs() <= 60)
                .map(|o| format!("{:.2}/{:.1}dB", scan::continuity_cost(&w[0], o).unwrap(), scan::noise_floor_jump(&w[0], o).unwrap_or(f64::NAN)))
                .collect();
            let noise = scan::noise_floor_jump(&w[0], &w[1]).unwrap_or(f64::NAN);
            println!("join {} -> {}: true {:.2}/{:.1}dB, rivals {:?}", w[0].name, w[1].name, good, noise, rivals);
        }
    }

    if let (Ok(out), Ok(pat)) = (std::env::var("PA_OUT_DIR"), std::env::var("PA_MERGE_MATCH")) {
        let rec = s.recordings.iter().find(|r| r.out_name.contains(&pat)).expect("no recording matches PA_MERGE_MATCH");
        let t = std::time::Instant::now();
        let sum = merge::run(&[rec], Path::new(&out), &AtomicBool::new(false), |_| {}).unwrap();
        println!("merge: {:?} in {:.2?}", sum, t.elapsed());
    }
}
