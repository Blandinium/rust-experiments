use crate::list::List;

impl<K: PartialEq, V> List<(K, V)> {
    /// Returns a reference to the value associated with `key` in the list of pairs, or `None` if not found.
    ///
    /// This method is only available for lists of `(K, V)` pairs where `K` can be compared for equality.
    ///
    /// # Examples
    ///
    /// ```
    /// use proplists::List;
    ///
    /// #[derive(PartialEq)]
    /// enum Key { A, B }
    ///
    /// let list = List::new((Key::B, 2)).prepend((Key::A, 1));
    /// assert_eq!(list.get_value(&Key::A), Some(&1));
    /// assert_eq!(list.get_value(&Key::B), Some(&2));
    /// ```
    pub fn get_value(&self, key: &K) -> Option<&V> {
        self.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Returns a list of references to all values associated with `key` in the list of pairs.
    ///
    /// # Examples
    ///
    /// ```
    /// use proplists::List;
    ///
    /// #[derive(PartialEq)]
    /// enum Key { A, B }
    ///
    /// let list = List::empty()
    ///     .prepend((Key::A, 3))
    ///     .prepend((Key::B, 2))
    ///     .prepend((Key::A, 1));
    /// let values = list.get_values(&Key::A);
    ///
    /// assert_eq!(values.iter().cloned().collect::<Vec<_>>(), vec![&1, &3]);
    /// ```
    pub fn get_values(&self, key: &K) -> List<&V> {
        self.iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v)
            .collect()
    }

    /// Returns a new list containing all pairs from the current list, except for those with the specified `key`.
    ///
    /// # Examples
    ///
    /// ```
    /// use proplists::List;
    ///
    /// #[derive(PartialEq, Clone, Debug)]
    /// enum Key { A, B }
    ///
    /// let list = List::empty()
    ///     .prepend((Key::A, 3))
    ///     .prepend((Key::B, 2))
    ///     .prepend((Key::A, 1));
    /// let deleted = list.delete(&Key::A);
    ///
    /// assert_eq!(deleted.iter().cloned().collect::<Vec<_>>(), vec![(Key::B, 2)]);
    /// ```
    pub fn delete(&self, key: &K) -> List<(K, V)>
    where
        K: Clone,
        V: Clone,
    {
        self.iter()
            .filter(|(k, _)| k != key)
            .cloned()
            .collect()
    }

}
