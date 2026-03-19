
/// Creates a [`List`] containing the specified elements.
///
/// # Examples
///
/// ```
/// use proplists::list;
///
/// // Create an empty list
/// let empty: proplists::List<i32> = list![];
/// assert!(empty.is_empty());
///
/// // Create a list with a single element
/// let single = list![1];
/// assert_eq!(single.hd(), Some(&1));
///
/// // Create a list with multiple elements
/// let multiple = list![1, 2, 3];
/// assert_eq!(multiple.hd(), Some(&1));
/// assert_eq!(multiple.tl().unwrap().hd(), Some(&2));
/// ```
#[macro_export]
macro_rules! list {
    () => {
        $crate::List::empty()
    };
    ($single:expr $(,)?) => {
        $crate::List::new($single)
    };
    ($head:expr, $($tail:expr),+ $(,)?) => {
        $crate::list!($($tail),+).prepend($head)
    };
}

/// Creates a [`List`] containing the specified elements wrapped in [`Arc`].
///
/// Useful for creating thread-safe lists of objects and avoid cloning
/// the objects on edits to the list.
///
/// # Examples
///
/// ```
/// use proplists::arc_list;
/// use std::sync::Arc;
///
/// // Create an empty list
/// let empty: proplists::List<Arc<i32>> = arc_list![];
/// assert!(empty.is_empty());
///
/// // Create a list with a single Arc-wrapped element
/// let single = arc_list![1];
/// assert_eq!(single.hd(), Some(&Arc::new(1)));
///
/// // Create a list with multiple Arc-wrapped elements
/// let multiple = arc_list![1, 2, 3];
/// assert_eq!(multiple.hd(), Some(&Arc::new(1)));
/// ```
#[macro_export]
macro_rules! arc_list {
    () => {
        $crate::List::empty()
    };
    ($single:expr $(,)?) => {
        $crate::List::new(std::sync::Arc::new($single))
    };
    ($head:expr, $($tail:expr),+ $(,)?) => {
        $crate::arc_list!($($tail),+).prepend(std::sync::Arc::new($head))
    };
}
