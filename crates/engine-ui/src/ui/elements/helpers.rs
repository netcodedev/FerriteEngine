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
