//! Which size of a panel did the user choose? Panels get sizes from the layout too (their first
//! frames, a window that is resized or maximised), and only a drag is worth remembering: a size
//! that changed while the pointer was down, taken when the pointer is let go.
use std::collections::HashMap;

#[derive(Default)]
pub struct SizeWatch {
    /// Per panel: the size last frame, and the size to remember if it was dragged.
    seen: HashMap<String, (f32, Option<f32>)>,
}

impl SizeWatch {
    /// One frame of panel `key` that is `extent` big. `Some(size)` once, on the frame the pointer
    /// is released after the panel was dragged to `size`.
    pub fn step(&mut self, key: &str, extent: f32, pointer_down: bool, released: bool) -> Option<f32> {
        let e = self.seen.entry(key.to_string()).or_insert((extent, None));
        if pointer_down && (e.0 - extent).abs() >= 1.0 {
            e.1 = Some(extent);
        }
        e.0 = extent;
        if released { e.1.take() } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_size_changed_by_a_drag_is_remembered_and_only_once() {
        let mut w = SizeWatch::default();
        // the layout gives the panel its first sizes, and the window is maximised: no drag
        assert_eq!(w.step("a", 160.0, false, false), None);
        assert_eq!(w.step("a", 544.0, false, false), None);
        // a click that moves nothing
        assert_eq!(w.step("a", 544.0, true, false), None);
        assert_eq!(w.step("a", 544.0, false, true), None);
        // a drag, then let go
        assert_eq!(w.step("a", 544.0, true, false), None);
        assert_eq!(w.step("a", 600.0, true, false), None);
        assert_eq!(w.step("a", 620.0, true, false), None);
        assert_eq!(w.step("a", 620.0, false, true), Some(620.0));
        // taken once
        assert_eq!(w.step("a", 620.0, false, true), None);
        // another panel is its own
        assert_eq!(w.step("b", 300.0, true, false), None);
        assert_eq!(w.step("b", 300.0, false, true), None);
    }
}
