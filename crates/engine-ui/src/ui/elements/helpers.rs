//! Shared helpers and macros for implementing `UIElement`.

/// Fills in the subtree-related `UIElement` methods for a widget that never
/// has children. Call inside an `impl UIElement for X { ... }` block.
#[macro_export]
macro_rules! leaf_children_methods {
    () => {
        fn add_child(&mut self, _child: Box<dyn $crate::ui::UIElement>) {
            panic!(concat!(
                stringify!(Self),
                " cannot have children"
            ));
        }
        fn add_child_to(
            &mut self,
            _parent: $crate::ui::element_handle::UIElementHandle,
            _child: Box<dyn $crate::ui::UIElement>,
        ) {
            panic!(concat!(stringify!(Self), " cannot have children"));
        }
        fn contains_child(
            &self,
            _handle: &$crate::ui::element_handle::UIElementHandle,
        ) -> bool {
            false
        }
        fn get_child(
            &self,
            _handle: &$crate::ui::element_handle::UIElementHandle,
        ) -> Option<&Box<dyn $crate::ui::UIElement>> {
            None
        }
        fn get_child_mut(
            &mut self,
            _handle: &$crate::ui::element_handle::UIElementHandle,
        ) -> Option<&mut Box<dyn $crate::ui::UIElement>> {
            None
        }
    };
}

/// `text` cut to its longest prefix that, followed by "…", is at most `max`
/// wide; just "…" when no prefix fits. For text that is wider than `max`.
/// The width grows with the prefix, so the longest one is found by a binary
/// search instead of trying every length.
pub(crate) fn ellipsized(text: &str, max: f32, measure: impl Fn(&str) -> f32) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut candidate = String::with_capacity(text.len() + 3);
    let set = |candidate: &mut String, keep: usize| {
        candidate.clear();
        candidate.extend(&chars[..keep]);
        candidate.push('\u{2026}');
    };
    // The prefixes tried keep fewer chars than the whole text.
    let (mut low, mut high) = (0, chars.len().saturating_sub(1));
    while low < high {
        let mid = (low + high + 1) / 2;
        set(&mut candidate, mid);
        if measure(&candidate) <= max {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    set(&mut candidate, low);
    candidate
}

#[cfg(test)]
mod tests {
    use super::ellipsized;

    /// The old way: every length, longest first.
    fn linear(text: &str, max: f32, measure: impl Fn(&str) -> f32) -> String {
        let chars: Vec<char> = text.chars().collect();
        (0..chars.len())
            .rev()
            .map(|keep| chars[..keep].iter().collect::<String>() + "\u{2026}")
            .find(|candidate| measure(candidate) <= max)
            .unwrap_or_else(|| "\u{2026}".into())
    }

    #[test]
    fn ellipsized_matches_trying_every_length() {
        // Uneven widths per char, like a proportional font.
        let measure = |s: &str| -> f32 {
            s.chars()
                .map(|c| match c {
                    'i' | 'l' => 3.0,
                    'm' | 'w' => 11.0,
                    '\u{2026}' => 9.0,
                    _ => 7.0,
                })
                .sum()
        };
        for text in ["", "a", "ab", "illumination", "mwmwmw ilil", "äöü – 漢字 path/to/a/file.glb"] {
            for max in [0.0, 5.0, 9.0, 10.0, 16.0, 30.0, 47.5, 60.0, 200.0] {
                assert_eq!(
                    ellipsized(text, max, measure),
                    linear(text, max, measure),
                    "{text:?} in {max}"
                );
            }
        }
    }
}
