use photocraft_engine::prefs::{Performance, Preferences};
use serde_json::json;

#[test]
fn old_preferences_keep_their_absolute_limit() {
    let p: Performance = serde_json::from_value(json!({"memoryUsageMb": 2048})).unwrap();
    assert_eq!(p.memory_usage_percent, None);
    assert_eq!(p.memory_usage_mb, 2048);
    assert_eq!(Performance::default().memory_usage_percent, Some(70));
}

#[test]
fn memory_preferences_validate_and_can_restore_absolute_mode() {
    let mut p = Preferences::default();
    assert!(p.set("performance.memoryUsagePercent", json!(96)).is_err());
    assert!(p.set("performance.memoryUsagePercent", json!(0)).is_err());
    assert!(p.set("performance.gpuMemoryMb", json!(-1)).is_err());
    p.set("performance.memoryUsagePercent", json!(75)).unwrap();
    assert_eq!(p.performance.memory_usage_percent, Some(75));
    p.set("performance.memoryUsagePercent", json!(null)).unwrap();
    assert_eq!(p.performance.memory_usage_percent, None);
    p.set("performance.gpuMemoryMb", json!(512)).unwrap();
    assert_eq!(p.performance.gpu_memory_mb, 512);
    // Main's GPU budget: 0 selects the automatic limit.
    p.set("performance.gpuMemoryMb", json!(0)).unwrap();
    assert_eq!(p.performance.gpu_memory_mb, 0);
}
