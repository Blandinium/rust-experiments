use crate::list::{List, ListCell};

/// An iterator over the elements of a `List`.
///
/// This struct is created by the `iter` method on `List` or by `into_iter` on `&List`.
///
/// # Examples
///
/// ```
/// use proplists::List;
/// let list = List::cons(1, &List::new(2));
/// let mut iter = list.iter();
/// assert_eq!(iter.next(), Some(&1));
/// ```
pub struct Iter<'a, T> {
    pub(crate) next: &'a ListCell<T>
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next {
            ListCell::Nil => None,
            ListCell::Cons(head, tail) => {
                self.next = tail.as_ref();
                Some(head)
            }
        }
    }
}

impl<'a, T> IntoIterator for &'a List<T> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    /// Returns an iterator over the list.
    ///
    /// # Examples
    ///
    /// ```
    /// use proplists::List;
    /// let list = List::cons(1, &List::new(2));
    /// for element in &list {
    ///     println!("{}", element);
    /// }
    /// ```
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<T> std::iter::FromIterator<T> for List<T> {
    /// Creates a `List` from an iterator.
    ///
    /// # Examples
    ///
    /// ```
    /// use proplists::List;
    /// let vec = vec![1, 2, 3];
    /// let list: List<i32> = vec.into_iter().collect();
    /// assert_eq!(list.hd(), Some(&1));
    /// assert_eq!(list.tl().unwrap().hd(), Some(&2));
    /// ```
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let items: Vec<T> = iter.into_iter().collect();
        items.into_iter()
            .rev()
            .fold(List::empty(), |acc, item| List::cons(item, &acc))
    }
}
