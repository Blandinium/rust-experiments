use std::sync::Arc;
use crate::listiter::Iter;

/// A singly linked list, similar to Lisp or Erlang lists.
///
/// Lists are read-only, and therefore safe to use from multiple threads,
/// on condition that T is thread-safe. Lists take ownership of their
/// data using an Arc reference, allowing clones of lists to share data.
pub struct List<T>(pub(crate) Arc<ListCell<T>>);

// We wrap ListCell into List, so we don't need to expose the Arc
// in the interface
pub(crate) enum ListCell<T> {
    Cons(T, Arc<ListCell<T>>),
    Nil,
}

impl<T> Clone for List<T> {
    /// Returns a copy of the list that shares its underlying data.
    ///
    /// # Examples
    ///
    /// ```
    /// use proplists::List;
    /// let list1 = List::new(42);
    /// let list2 = list1.clone();
    /// assert_eq!(list1.hd(), list2.hd());
    /// ```
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> Default for List<T> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for List<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T: PartialEq> PartialEq for List<T> {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

impl<T: Eq> Eq for List<T> {}

impl<T> List<T> {
    /// Returns an empty list.
    ///
    /// # Examples
    ///
    /// ```
    /// use proplists::List;
    /// let list: List<i32> = List::empty();
    /// assert!(list.hd().is_none());
    /// ```
    pub fn empty() -> Self {
        Self(Arc::new(ListCell::Nil))
    }
    /// Creates a new list with a single element.
    ///
    /// # Examples
    ///
    /// ```
    /// use proplists::List;
    /// let list = List::new(42);
    /// assert_eq!(list.hd(), Some(&42));
    /// ```
    pub fn new(data: T) -> Self {
        Self::cons(data, &Self::empty())
    }
    /// Returns a new list with `data` at the head, followed by the contents of `list`.
    ///
    /// # Examples
    ///
    /// ```
    /// use proplists::List;
    /// let list1 = List::new(2);
    /// let list2 = List::cons(1, &list1);
    /// assert_eq!(list2.hd(), Some(&1));
    /// assert_eq!(list2.tl().unwrap().hd(), Some(&2));
    /// ```
    pub fn cons(data: T, list: &Self) -> Self {
        Self(Arc::new(ListCell::Cons(data, Arc::clone(&list.0))))
    }
    /// Returns a reference to the first element of the list (the head), or `None` if it is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use proplists::List;
    /// let list = List::new(1);
    /// assert_eq!(list.hd(), Some(&1));
    ///
    /// let empty: List<i32> = List::empty();
    /// assert_eq!(empty.hd(), None);
    /// ```
    pub fn hd(&self) -> Option<&T> {
        match &*self.0 {
            ListCell::Cons(head, _) => Some(head),
            ListCell::Nil => None,
        }
    }
    /// Returns the rest of the list after the first element (the tail), or `None` if the list is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use proplists::List;
    /// let list = List::cons(1, &List::new(2));
    /// let tail = list.tl().unwrap();
    /// assert_eq!(tail.hd(), Some(&2));
    /// ```
    pub fn tl(&self) -> Option<Self> {
        match &*self.0 {
            ListCell::Cons(_, tail) => Some(Self(Arc::clone(tail))),
            ListCell::Nil => None,
        }
    }
    /// Returns `true` if the list is empty, and `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```
    /// use proplists::List;
    /// let empty: List<i32> = List::empty();
    /// assert!(empty.is_empty());
    ///
    /// let list = List::new(42);
    /// assert!(!list.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        match &*self.0 {
            ListCell::Cons(_, _) => false,
            ListCell::Nil => true,
        }
    }

    /// Returns an iterator over the list.
    ///
    /// # Examples
    ///
    /// ```
    /// use proplists::List;
    /// let list = List::cons(1, &List::new(2));
    /// let mut iter = list.iter();
    /// assert_eq!(iter.next(), Some(&1));
    /// assert_eq!(iter.next(), Some(&2));
    /// assert_eq!(iter.next(), None);
    /// ```
    pub fn iter(&self) -> Iter<'_, T> {
        Iter { next: &*self.0 }
    }
}

impl<T: Clone> List<T> {
    /// Returns a new list with the (cloned) elements in reverse order.
    ///
    /// # Examples
    ///
    /// ```
    /// use proplists::List;
    /// let list = List::cons(1, &List::new(2));
    /// let reversed = list.reverse();
    /// assert_eq!(reversed.hd(), Some(&2));
    /// assert_eq!(reversed.tl().unwrap().hd(), Some(&1));
    /// ```
    pub fn reverse(&self) -> Self {
        self.iter()
            .cloned()
            .fold(List::empty(), |acc, item| List::cons(item, &acc))
    }
}
