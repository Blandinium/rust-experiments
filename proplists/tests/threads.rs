use std::sync::mpsc;
use std::thread;
use proplists::List;

#[test]
fn read_shared_tail_from_multiple_threads() {
    let (tx, rx) = mpsc::channel();
    let tail: List<i32> = List::new(1).prepend(5).prepend(10);

    let mut handles = Vec::new();
    for n in 0..10 {
        let tx = tx.clone();
        let list = tail.prepend(n);
        handles.push(thread::spawn(move || {
            let sum: i32 = list.iter().sum();
            tx.send(sum).unwrap();
        }));
    }
    drop(tx);

    let total: i32 = rx.iter().sum();
    assert_eq!(total, 205);
    for handle in handles {
        handle.join().unwrap();
    }
}
