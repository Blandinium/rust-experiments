use crate::actor_message::{ActorMessage, ActorRef, MsgCtx};
use dashmap::DashMap;
use log::warn;
use std::any::Any;
use std::collections::{VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc::{self, Receiver, Sender}, Arc, Condvar, Mutex};

pub (crate) struct ActorQueue {
    // The mutes on the counter, serves as a lock on any actor state change
    counters: DashMap<ActorRef, Arc<Mutex<usize>>>,
    // Actors without pending messages
    idle: DashMap<ActorRef, ()>,
    // Actors with pending messages
    queue: Arc<Mutex<VecDeque<ActorRef>>>,
    senders: DashMap<ActorRef, Sender<ActorMessage>>,
    receivers: DashMap<ActorRef, Arc<Mutex<Receiver<ActorMessage>>>>,
    // Used to generate unique actor references
    next_id: AtomicU64,
    // Used to block when there are no messages to process
    work_condvar: Condvar,
}

impl ActorQueue {
    pub fn new() -> Self {
        Self {
            counters: DashMap::new(),
            idle: DashMap::new(),
            senders: DashMap::new(),
            receivers: DashMap::new(),
            queue: Arc::new(Mutex::new(VecDeque::new())),
            next_id: AtomicU64::new(0),
            work_condvar: Condvar::new(),
        }
    }

    pub fn new_actor(&self) -> ActorRef {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let actor_ref = ActorRef(id);
        let (tx, rx) = mpsc::channel();
        self.senders.insert(actor_ref, tx);
        self.receivers.insert(actor_ref, Arc::new(Mutex::new(rx)));
        self.idle.insert(actor_ref, ());
        self.counters.insert(actor_ref, Arc::new(Mutex::new(0)));
        actor_ref
    }

    pub fn send(&self, to: ActorRef, from: Option<ActorRef>, msg: Box<dyn Any + Send>) -> bool {
        let counter_entry = match self.counters.get(&to) {
            Some(c) => c,
            None => {
                warn!("send: no counter found for actor {:?}", to);
                return false;
            }
        };

        // We lock the mutex for this actor, and hold the lock till we are done
        let mut counter = match counter_entry.lock() {
            Ok(c) => c,
            Err(_) => {
                warn!("send: counter mutex poisoned for actor {:?}", to);
                return false;
            }
        };

        let sender = match self.senders.get(&to) {
            Some(s) => s,
            None => {
                warn!("send: no sender found for actor {:?}", to);
                return false;
            }
        };

        if sender.send((msg, MsgCtx { from, to })).is_err() {
            warn!("send: failed to send message to actor {:?}", to);
            return false;
        }

        if self.idle.remove(&to).is_some() {
            if let Ok(mut q) = self.queue.lock() {
                q.push_back(to);
            } else {
                warn!("send: queue mutex poisoned");
                return false;
            }
        }

        *counter += 1;
        drop(counter);
        self.work_condvar.notify_one();
        true
    }

    /// Polls the internal queue for a message associated with an actor.
    ///
    /// This method retrieves the next `(ActorRef, ActorMessage)` tuple from the queue,
    /// if available. If there are no messages, the function either waits for a message
    /// to arrive or returns `None` if there are no actors to process.
    ///
    /// ### Behavior:
    /// - If the internal counter map for the actor is missing, a warning is logged,
    ///   and the polling process is restarted (recursive call to `poll`).
    /// - If the mutex managing the actor's counter is poisoned, a warning is logged,
    ///   and the function returns `None`.
    /// - If the actor's receiver unexpectedly lacks a message or does not exist, the function
    ///   will panic due to inconsistent internal states which violate assumptions.
    ///
    /// ### Returns:
    /// - `Some((ActorRef, ActorMessage))`: A tuple containing the reference to the actor and the
    ///   associated message, if available.
    /// - `None`: If there are currently no actors or no messages to process.
    ///
    /// ### Panics:
    /// - The function panics if the internal state is inconsistent, such as when an actor
    ///   is in the queue without a receiver or a message.
    ///
    /// ### Threading & Synchronization:
    /// - Operates on an internally managed `queue` protected by a mutex.
    /// - Utilizes a condition variable (`work_condvar`) to block the thread and wait until
    ///   a message is available.
    /// - Works with actor-specific `counter` mutexes to ensure safe processing.
    pub fn poll(&self) -> Option<(ActorRef, ActorMessage)> {
        let actor_ref = loop {
            // When there are no actors, there is nothing to wait for
            if self.counters.is_empty() {
                return None;
            }

            let mut queue = self.queue.lock().unwrap();

            // Wait until there is work, unless all actors are gone
            while queue.is_empty() {
                if self.counters.is_empty() {
                    return None;
                }

                queue = self.work_condvar.wait(queue).unwrap();
            }

            break queue.pop_front().unwrap();
        };
        
        let counter_entry = match self.counters.get(&actor_ref) {
            Some(c) => c,
            None => {
                warn!("pop: no counter found for actor {:?}", actor_ref);
                // This should mean the actor was deleted. We just try again
                return self.poll();
            }
        };
        // We lock the mutex for this actor, and hold the lock till we are done
        let mut counter = match counter_entry.lock() {
            Ok(c) => c,
            Err(_) => {
                warn!("pop: counter mutex poisoned for actor {:?}", actor_ref);
                return None;
            }
        };
        let receiver = self.receivers.get(&actor_ref)
            .expect("queued actor had no receiver; inconsistent internal state");
        let msg = receiver.lock().unwrap().try_recv()
            .expect("queued actor had no message; invariant violated");
        *counter -= 1;
        Some((actor_ref, msg))
    }

    pub fn return_to_queue(&self, actor_ref: ActorRef) {
        let counter_entry = match self.counters.get(&actor_ref) {
            Some(c) => c,
            None => {
                // We did not find a counter. This should mean the actor was deleted while running
                return;
            }
        };
        // We lock the mutex for this actor, and hold the lock till we are done
        let counter = match counter_entry.lock() {
            Ok(c) => c,
            Err(_) => {
                warn!("return_to_queue: counter mutex poisoned for actor {:?}", actor_ref);
                return;
            }
        };
        let should_notify = if *counter > 0 {
            self.queue.lock().unwrap().push_back(actor_ref);
            true
        } else {
            self.idle.insert(actor_ref, ());
            false
        };
        drop(counter); // Only at the end, we free the lock
        if should_notify {
            self.work_condvar.notify_one();
        }
    }
    
    pub fn remove_actor(&self, actor_ref : &ActorRef) {
        let counter_entry = match self.counters.get(&actor_ref) {
            Some(c) => c,
            None => {
                // We did not find a counter. This should mean the actor was already deleted
                return;
            }
        };
        // We lock the mutex for this actor, and hold the lock till we are done
        let counter = match counter_entry.lock() {
            Ok(c) => c,
            Err(_) => {
                warn!("remove_actor: counter mutex poisoned for actor {:?}", actor_ref);
                return;
            }
        };
        self.senders.remove(&actor_ref);
        self.receivers.remove(&actor_ref);
        self.idle.remove(&actor_ref);
        self.queue.lock().unwrap().retain(|&a| a != *actor_ref);
        drop(counter) // Only at the end, we free the lock
    }
    
    pub fn shutdown(&self) {
        // Prevent any future poll
        self.counters.clear();
        // Stop all polling threads
        self.work_condvar.notify_all();
    }
}
