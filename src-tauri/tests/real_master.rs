//! Mastering against real files (not run by default).
//!
//!   PA_MASTER_DIR=/path cargo test --release --test real_master -- --ignored --nocapture
//!   optional: PA_MASTER_OUT=/tmp/out PA_MASTER_MATCH=part-of-name

use prepare_audio_lib::master;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

#[test]
#[ignore]
fn real_master() {
    let dirs: Vec<PathBuf> = std::env::var("PA_MASTER_DIR").expect("PA_MASTER_DIR").split('|').map(PathBuf::from).collect();
    let t = std::time::Instant::now();
    let plan = master::analyze(&dirs, &AtomicBool::new(false), &mut |_| {}).unwrap();
    println!("analysis {:.1?}: {} files, out {}, engine {}", t.elapsed(), plan.files.len(), plan.default_out_dir, plan.engine);
    for f in &plan.files {
        let l = f.loudness.as_ref();
        println!(
            "FILE {:>2} {:<48} {:<36} {:>7.0}s I {:>6.1} TP {:>5.1} LRA {:>4.1} gain {:>+6.1} limiter {:>4.1} dji={} {:?}",
            f.id, f.name, f.format, f.duration, l.map_or(f64::NAN, |l| l.lufs), l.map_or(f64::NAN, |l| l.true_peak), l.map_or(f64::NAN, |l| l.lra),
            f.gain_db.unwrap_or(f64::NAN), f.limited_db, f.dji_part, f.note
        );
    }
    if let (Ok(out), Ok(pat)) = (std::env::var("PA_MASTER_OUT"), std::env::var("PA_MASTER_MATCH")) {
        let ids: Vec<usize> = plan.files.iter().filter(|f| f.name.contains(&pat)).map(|f| f.id).collect();
        let t = std::time::Instant::now();
        let sum = master::write(&plan, &ids, Path::new(&out), &AtomicBool::new(false), |_| {}).unwrap();
        println!("write {:.1?}", t.elapsed());
        for o in &sum.outcomes {
            println!("OUT {:?} {:?} {:?} {:?}", o.status, o.path, o.result, o.message);
        }
    }
}
