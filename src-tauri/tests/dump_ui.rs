//! Writes real scan and sync results in every interface language as JSON, for the browser
//! tests and the website screenshots (not run by default):
//!   PA_DUMP_SCAN=/path/day PA_DUMP_SYNC=/path/day/tracks PA_DUMP_DIR=/tmp/out \
//!   cargo test --release --test dump_ui -- --ignored --nocapture
//! Produces scan-<lang>.json and plan-<lang>.json ({ plan, peaks }) for de, en, fr, it.

use prepare_audio_lib::{i18n, scan, sync};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

#[test]
#[ignore]
fn dump_ui_data_in_every_language() {
    let scan_dirs: Vec<PathBuf> = std::env::var("PA_DUMP_SCAN").expect("PA_DUMP_SCAN").split('|').map(PathBuf::from).collect();
    let sync_dirs: Vec<PathBuf> = std::env::var("PA_DUMP_SYNC").expect("PA_DUMP_SYNC").split('|').map(PathBuf::from).collect();
    let out = PathBuf::from(std::env::var("PA_DUMP_DIR").expect("PA_DUMP_DIR"));
    std::fs::create_dir_all(&out).unwrap();
    for lang in ["de", "en", "fr", "it"] {
        assert!(i18n::set(lang));
        let s = scan::scan(&scan_dirs, &scan::Options::default()).unwrap();
        std::fs::write(out.join(format!("scan-{lang}.json")), serde_json::to_string(&s).unwrap()).unwrap();
        let plan = sync::analyze(&sync_dirs, &AtomicBool::new(false), &mut |_| {}).unwrap();
        let peaks: Vec<Vec<u8>> = plan.tracks.iter().map(|t| sync::peaks(&plan, t.id, 0.0, t.duration, 20_000)).collect();
        let v = serde_json::json!({ "plan": plan, "peaks": peaks });
        std::fs::write(out.join(format!("plan-{lang}.json")), serde_json::to_string(&v).unwrap()).unwrap();
        println!("{lang}: {} recordings, {} sync items", s.recordings.len(), plan.items.len());
    }
}
