use photocraft_compose::Buffer;
use photocraft_geom::Rect;
use photocraft_raster::scratch;

/// Restores idle release even when an assertion fails.
struct IdleReleaseOff;
impl Drop for IdleReleaseOff {
    fn drop(&mut self) {
        scratch::release();
        scratch::set_idle_release(true);
    }
}

/// Runs in its own binary so other compositor tests cannot consume the pooled allocation.
#[test]
fn dropped_buffer_storage_is_reused() {
    scratch::set_idle_release(false);
    let _idle = IdleReleaseOff;
    scratch::release();
    let rect = Rect::from_xywh(0, 0, 256, 256);
    let buf = Buffer::transparent(rect);
    let ptr = buf.px.as_ptr();
    drop(buf);
    let reused = Buffer::transparent(rect);
    assert_eq!(reused.px.as_ptr(), ptr);
    assert!(reused.px.iter().all(|p| *p == [0.0; 4]));
}
