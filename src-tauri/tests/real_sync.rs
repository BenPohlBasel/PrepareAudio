//! Read-only check of the sync analysis against real tracks (not run by default).
//!
//!   PA_SYNC_DIR="/path/tracks" cargo test --release --test real_sync -- --ignored --nocapture
//!   optional: PA_SYNC_JSON=/tmp/plan.json, PA_SYNC_OUT=/tmp/out PA_SYNC_MATCH=stereo

use prepare_audio_lib::sync;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

fn hms(sec: f64) -> String {
    let s = sec.round() as i64;
    format!("{:02}:{:02}:{:02}", s / 3600 % 24, s % 3600 / 60, s % 60)
}

#[test]
#[ignore]
fn real_sync() {
    let dirs: Vec<PathBuf> = std::env::var("PA_SYNC_DIR").expect("PA_SYNC_DIR").split('|').map(PathBuf::from).collect();
    let t = std::time::Instant::now();
    let plan = sync::analyze(&dirs, &AtomicBool::new(false), &mut |_| {}).unwrap();
    println!(
        "analysis {:.1?}: source {}, {} tracks ({:?}), {} pairs, {} items, out {}",
        t.elapsed(),
        plan.source,
        plan.tracks.len(),
        plan.labels,
        plan.pairs.len(),
        plan.items.len(),
        plan.default_out_dir
    );
    for p in &plan.pairs {
        let (a, b) = (&plan.tracks[p.a], &plan.tracks[p.b]);
        println!(
            "PAIR {} {} <-> {} {}: ok={} offset {:+.4} s drift {:+.2} ppm resid {:.1} ms coarse z {:.1} windows {}/{} {:?}",
            a.label, a.start, b.label, b.start, p.ok, p.offset, p.drift_ppm, p.resid_ms, p.coarse_z, p.n_good, p.n_windows, p.note
        );
        let line: String = p.frames.iter().map(|f| if !f.active { ' ' } else if f.hit { '#' } else if f.lag_ms.abs() <= 20 { '+' } else { '.' }).collect();
        if !line.is_empty() {
            println!("   {line}");
        }
        for ph in &p.phases {
            println!(
                "   {:8} {}–{}  hits {:?}  msc {:?}",
                ph.kind,
                hms(a.clock0 + ph.start),
                hms(a.clock0 + ph.end),
                ph.hit_share.map(|v| (v * 100.0).round()),
                ph.msc.map(|v| (v * 1000.0).round() / 1000.0)
            );
        }
    }
    for it in &plan.items {
        println!("ITEM {:>3} {:6} {:9} {} {}–{} {:>8.1}s {}", it.id, it.kind, it.reason, it.day, it.start, it.end, it.duration, it.name);
    }
    if let Ok(json) = std::env::var("PA_SYNC_JSON") {
        std::fs::write(&json, serde_json::to_string(&plan).unwrap()).unwrap();
    }
    if let (Ok(out), Ok(pat)) = (std::env::var("PA_SYNC_OUT"), std::env::var("PA_SYNC_MATCH")) {
        let ids: Vec<usize> = plan.items.iter().filter(|i| i.name.contains(&pat)).map(|i| i.id).collect();
        let t = std::time::Instant::now();
        let sum = sync::write(&plan, &ids, Path::new(&out), &AtomicBool::new(false), |_| {}).unwrap();
        println!("write {:.1?}: {:?}", t.elapsed(), sum.outcomes);
    }
}
