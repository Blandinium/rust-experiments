# rust-experiments
Some experiments while learning Rust.

## proplist

A basic implementation of Erlang-style property lists. Create an imutable singly-linked list with the macro `list![1,2,3]`, or use `arc_list![1, 2, 3]` if you want the values to live on the heap.

A property list is a list of key-value pairs, with some methods similar to these you'd expect on a map. You can create one like this: `list![("a", 1),("b", 2),("a", 3)]`.


## Actrs

Very much a work in progress

An attempt at implementing actors similar to gen_server in Erlang. Actors process messages. The method that processes the message receives the actor state from the framework, and returns it again. The framework decides how to schedule a large amount of actors over a smaller number of threads.
