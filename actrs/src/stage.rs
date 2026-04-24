use crate::actor::{Actor, ActorResult, AnyActor};
use crate::actor_message::{ActorMessage, ActorRef};
use crate::actor_queue::ActorQueue;
use dashmap::DashMap;
use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread::{self, JoinHandle};

/// A `Stage` manages a set of actors and schedules their message processing
/// across a fixed pool of worker threads.
///
/// It provides batching of messages to improve throughput and allows
/// actors to be added and signaled for shutdown.
///
/// # Examples
///
/// ```
/// use actrs::Stage;
/// use std::time::Duration;
///
/// let stage = Stage::new(4);
/// // add actors, send messages...
/// stage.shutdown();
/// ```
pub struct Stage {
    actors: DashMap<ActorRef, Arc<dyn AnyActor>>,
    states: DashMap<ActorRef, Box<dyn Any + Send + Sync>>,
    queue: ActorQueue,
    shutdown_flag: Arc<AtomicBool>,
    worker_threads: Mutex<Vec<JoinHandle<()>>>,
    empty_condvar: Condvar,
    completion_lock: Mutex<bool>,
}

impl Stage {
    /// Creates a new `Stage` with the specified configuration.
    ///
    /// - `num_threads`: Number of worker threads to spawn.
    pub fn new(num_threads: usize) -> Arc<Self> {
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let stage_arc = Arc::new(Self {
            actors: DashMap::new(),
            states: DashMap::new(),
            queue: ActorQueue::new(),
            shutdown_flag: shutdown_flag.clone(),
            worker_threads: Mutex::new(Vec::new()),
            empty_condvar: Condvar::new(),
            completion_lock: Mutex::new(true),
        });

        let mut threads = Vec::new();
        for _ in 0..num_threads {
            let stage_weak = Arc::downgrade(&stage_arc);
            let shutdown_clone = shutdown_flag.clone();
            let handle = thread::spawn(move || loop {
                let next = {
                    let stage = match stage_weak.upgrade() {
                        Some(s) => s,
                        None => return,
                    };
                    loop {
                        if shutdown_clone.load(Ordering::Relaxed) {
                            return;
                        }
                        if let Some(work) = stage.queue.poll() {
                            break Some(work);
                        }
                    }
                };

                if let Some((actor_ref, msg)) = next {
                    if let Some(stage) = stage_weak.upgrade() {
                        stage.run_actor(actor_ref, msg);
                    } else {
                        return;
                    }
                }
            });
            threads.push(handle);
        }

        {
            let mut worker_threads = stage_arc.worker_threads.lock().unwrap();
            *worker_threads = threads;
        }

        stage_arc
    }

    /// Returns the number of currently active actors in the stage.
    pub fn actor_count(&self) -> usize {
        self.actors.len()
    }

    /// Signals all worker threads to stop and joins them.
    ///
    /// This should be called once the stage is no longer needed.
    pub fn shutdown(&self) {
        self.shutdown_flag.store(true, Ordering::Relaxed);
        self.queue.shutdown();
        let mut threads = self.worker_threads.lock().unwrap();
        for handle in threads.drain(..) {
            let _ = handle.join();
        }
        self.actors.clear();
        self.states.clear();
        if let Ok(mut completed) = self.completion_lock.lock() {
            *completed = true;
        }
        self.empty_condvar.notify_all();
    }
}

impl Stage {
    pub fn wait_for_completion(&self) {
        let mut completed = self.completion_lock.lock().unwrap();
        while !*completed {
            completed = self.empty_condvar.wait(completed).unwrap();
        }
    }

    fn run_actor(
        &self,
        actor_ref: ActorRef,
        msg: ActorMessage,
    ) {
        let actor = self.actors.get(&actor_ref).map(|a| a.value().clone());

        if let Some(actor) = actor {
            let mut state = self
                .states
                .remove(&actor_ref)
                .map(|(_, s)| s)
                .expect("actor state not found");

            match actor.consume_any_message(msg.0, msg.1, state) {
                ActorResult::Ok(new_state) => {
                    state = new_state;
                }
                ActorResult::Stop | ActorResult::Error(_) => {
                    self.actors.remove(&actor_ref);
                    self.queue.remove_actor(&actor_ref);
                    if self.actors.is_empty() {
                        if let Ok(mut completed) = self.completion_lock.lock() {
                            *completed = true;
                        }
                        self.empty_condvar.notify_all();
                    }
                    return;
                }
            }

            self.states.insert(actor_ref, state);
            self.queue.return_to_queue(actor_ref);
        }
    }

    /// Sends a message to the specified actor.
    ///
    /// - `to`: Reference to the target actor.
    /// - `from`: Optional reference to the sender actor.
    /// - `msg`: The message to send.
    pub fn send(&self, to: ActorRef, from: Option<ActorRef>, msg: Box<dyn Any + Send>) {
        self.queue.send(to, from, msg);
    }

    /// Adds a new actor to the stage and performs its initialization.
    ///
    /// The actor's `set_handle` is called before its `handle_init`.
    ///
    /// - `actor`: The actor instance.
    /// - `init_param`: The parameter passed to the actor's `handle_init`.
    ///
    /// Returns the actor's new `ActorRef`.
    pub fn add_actor<A>(self: &Arc<Self>, mut actor: A, init_param: A::I) -> ActorRef
    where
        A: Actor + StageAware + 'static + Send + Sync,
    {
        let actor_ref = self.queue.new_actor();
        let handle = ActorHandle::new(self, actor_ref);
        actor.set_handle(handle);

        match actor.handle_init(init_param) {
            ActorResult::Ok(initial_state) => {
                if let Ok(mut completed) = self.completion_lock.lock() {
                    *completed = false;
                }
                self.states.insert(actor_ref, Box::new(initial_state));
                self.actors.insert(actor_ref, Arc::new(actor));
                actor_ref
            }
            ActorResult::Stop | ActorResult::Error(_) => {
                // If initialization fails, we remove the actor again
                self.queue.remove_actor(&actor_ref);
                if self.actors.is_empty() {
                    if let Ok(mut completed) = self.completion_lock.lock() {
                        *completed = true;
                    }
                    self.empty_condvar.notify_all();
                }
                // We still return an ActorRef, but it will be dead.
                actor_ref
            }
        }
    }
}

/// A handle that allows an actor to interact with the stage and itself.
#[derive(Clone)]
pub struct ActorHandle {
    stage: Weak<Stage>,
    self_ref: ActorRef,
}

/// A trait for actors that need to hold a handle to themselves and the stage.
///
/// This trait is automatically implemented when using the `#[actor]` macro.
pub trait StageAware {
    /// Sets the handle for the actor. This is called by the stage during actor creation.
    fn set_handle(&mut self, handle: ActorHandle);
}

impl ActorHandle {
    /// Creates a new `ActorHandle`. This is primarily used internally by the stage.
    pub fn new(stage: &Arc<Stage>, self_ref: ActorRef) -> Self {
        Self {
            stage: Arc::downgrade(stage),
            self_ref,
        }
    }

    /// Returns the reference of the actor that owns this handle.
    pub fn self_ref(&self) -> ActorRef {
        self.self_ref
    }

    /// Sends a message to another actor on the same stage.
    ///
    /// # Errors
    /// Returns an error if the stage has been dropped.
    pub fn send(
        &self,
        to: ActorRef,
        msg: Box<dyn Any + Send>,
    ) -> Result<(), &'static str> {
        let stage = self.stage.upgrade().ok_or("stage no longer exists")?;
        stage.send(to, Some(self.self_ref), msg);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MsgCtx;
    use std::time::{Duration, Instant};

    struct MyActor {
        _handle: Option<ActorHandle>,
    }

    impl MyActor {
        fn new() -> Self {
            Self { _handle: None }
        }
    }

    impl Actor for MyActor {
        type S = i32;
        type M = i32;
        type I = i32;

        fn consume_message(&self, msg: Box<i32>, _ctx: MsgCtx, state: i32) -> ActorResult<i32> {
            ActorResult::Ok(state + *msg)
        }

        fn handle_init(&self, param: i32) -> ActorResult<i32> {
            ActorResult::Ok(param)
        }
    }

    impl StageAware for MyActor {
        fn set_handle(&mut self, handle: ActorHandle) {
            self._handle = Some(handle);
        }
    }

    fn wait_for_state(stage: &Stage, actor_ref: ActorRef, expected: i32) {
        let deadline = Instant::now() + Duration::from_millis(500);

        loop {
            if let Some(state) = stage.states.get(&actor_ref) {
                if let Some(state) = state.downcast_ref::<i32>() {
                    if *state == expected {
                        return;
                    }
                }
            }

            if Instant::now() >= deadline {
                break;
            }

            let completed = stage.completion_lock.lock().unwrap();
            let _ = stage.empty_condvar.wait_timeout(completed, Duration::from_millis(5)).unwrap();
        }

        let state = stage.states.get(&actor_ref).unwrap();
        let state = state.downcast_ref::<i32>().unwrap();
        assert_eq!(*state, expected);
    }

    #[test]
    fn test_stage_send_accumulates_messages() {
        let stage = Stage::new(2);
        let actor_ref = stage.add_actor(MyActor::new(), 0);

        stage.send(actor_ref, None, Box::new(10));
        stage.send(actor_ref, None, Box::new(20));

        wait_for_state(&stage, actor_ref, 30);
    }

    #[test]
    fn test_stage_batches_more_than_limit() {
        let stage = Stage::new(2);
        let actor_ref = stage.add_actor(MyActor::new(), 0);

        for _ in 0..11 {
            stage.send(actor_ref, None, Box::new(1));
        }

        wait_for_state(&stage, actor_ref, 11);
    }

    #[test]
    fn test_multiple_actors_unique_refs() {
        let stage = Stage::new(1);
        let ref1 = stage.add_actor(MyActor::new(), 0);
        let ref2 = stage.add_actor(MyActor::new(), 0);

        assert_ne!(ref1, ref2);
    }

    #[test]
    fn test_stage_drop_shuts_down_threads() {
        use std::sync::atomic::AtomicUsize;
        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);
        DROP_COUNT.store(0, Ordering::SeqCst);

        struct DropActor;
        impl Actor for DropActor {
            type M = ();
            type S = ();
            type I = ();
            fn consume_message(&self, _msg: Box<()>, _ctx: MsgCtx, _state: ()) -> ActorResult<()> {
                ActorResult::Ok(())
            }
            fn handle_init(&self, _param: ()) -> ActorResult<()> {
                ActorResult::Ok(())
            }
        }
        impl StageAware for DropActor {
            fn set_handle(&mut self, _handle: ActorHandle) {}
        }
        impl Drop for DropActor {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }

        {
            let stage = Stage::new(2);
            stage.add_actor(DropActor, ());
            // stage goes out of scope here
        }

        // Wait a bit to see if DROP_COUNT increases.
        // It should now increase because Drop for Stage calls shutdown, which joins threads,
        // and threads held Weak<Stage> so the reference count could reach zero.
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1, "Actors should be dropped because Stage was dropped and shut down");
    }
}
