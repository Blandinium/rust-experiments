use dashmap::DashMap;
use std::any::Any;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

pub trait StageAware {
    fn set_handle(&mut self, handle: ActorHandle);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActorRef(pub u64);

#[derive(Debug, Clone, Copy)]
pub struct MsgCtx {
    pub from: Option<ActorRef>,
    pub to: ActorRef,
}

/// A trait that can handle any message by downcasting.
pub trait AnyActor: Send + Sync {
    fn consume_any_message(&self, msg: Box<dyn Any + Send>, ctx: MsgCtx, state: Box<dyn Any + Send + Sync>) -> Box<dyn Any + Send + Sync>;
    fn init_any(&self, param: Box<dyn Any + Send>) -> Box<dyn Any + Send + Sync>;
}

/// The core Actor trait.
/// - `S`: The Actor's internal state type.
/// - `M`: The message type that this actor handles.
/// - `I`: The initialization parameter type.
pub trait Actor: AnyActor
where
    Self::M: Any + Send,
    Self::S: Any + Send + Sync,
    Self::I: Any + Send,
{
    /// The Actor's internal state type.
    type S;
    /// The message type that this actor handles.
    type M;
    /// The initialization parameter type.
    type I;

    fn consume_message(&self, msg: Box<Self::M>, ctx: MsgCtx, state: Self::S) -> Self::S;
    fn handle_init(&self, param: Self::I) -> Self::S;
}

impl<A> AnyActor for A
where
    A: Actor + Send + Sync,
    A::M: Any + Send,
    A::S: Any + Send + Sync,
    A::I: Any + Send,
{
    fn consume_any_message(&self, msg: Box<dyn Any + Send>, ctx: MsgCtx, state: Box<dyn Any + Send + Sync>) -> Box<dyn Any + Send + Sync> {
        let msg = msg.downcast::<A::M>().expect("invalid message type for actor");
        let state = state.downcast::<A::S>().expect("invalid state type for actor");
        let new_state = self.consume_message(msg, ctx, *state);
        Box::new(new_state)
    }

    fn init_any(&self, param: Box<dyn Any + Send>) -> Box<dyn Any + Send + Sync> {
        let param = param.downcast::<A::I>().expect("invalid initialization parameter for actor");
        Box::new(self.handle_init(*param))
    }
}

type ActorMessage = (Box<dyn Any + Send>, MsgCtx);
type ActorQueue = Arc<Mutex<VecDeque<(ActorRef, Receiver<ActorMessage>, ActorMessage)>>>;

pub struct Stage {
    actors: DashMap<ActorRef, Arc<dyn AnyActor>>,
    states: DashMap<ActorRef, Box<dyn Any + Send + Sync>>,
    unqueued: DashMap<ActorRef, Mutex<Receiver<ActorMessage>>>,
    senders: DashMap<ActorRef, Sender<ActorMessage>>,
    queue: ActorQueue,
    next_id: AtomicU64,
    max_batch_messages: usize,
    max_batch_time: Duration,
}

impl Stage {
    pub fn new(num_threads: usize, max_batch_messages: usize, max_batch_time: Duration) -> Arc<Self> {
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let stage_arc = Arc::new(Self {
            actors: DashMap::new(),
            states: DashMap::new(),
            unqueued: DashMap::new(),
            senders: DashMap::new(),
            queue: queue.clone(),
            next_id: AtomicU64::new(1),
            max_batch_messages,
            max_batch_time,
        });

        for _ in 0..num_threads {
            let stage_clone = Arc::clone(&stage_arc);
            thread::spawn(move || loop {
                let next = {
                    let mut q = stage_clone.queue.lock().unwrap();
                    q.pop_front()
                };

                if let Some((actor_ref, receiver, (msg, ctx))) = next {
                    stage_clone.run_actor(actor_ref, receiver, msg, ctx);
                } else {
                    thread::sleep(Duration::from_millis(10));
                }
            });
        }

        stage_arc
    }

    fn run_actor(
        &self,
        actor_ref: ActorRef,
        receiver: Receiver<ActorMessage>,
        msg: Box<dyn Any + Send>,
        ctx: MsgCtx,
    ) {
        let actor = self.actors.get(&actor_ref).map(|a| a.value().clone());

        if let Some(actor) = actor {
            let state = self
                .states
                .remove(&actor_ref)
                .map(|(_, s)| s)
                .expect("actor state not found");

            let deadline = Instant::now() + self.max_batch_time;
            let mut processed = 0usize;
            let mut current_state = actor.consume_any_message(msg, ctx, state);
            processed += 1;

            loop {
                if processed >= self.max_batch_messages || Instant::now() >= deadline {
                    break;
                }

                match receiver.try_recv() {
                    Ok((msg, ctx)) => {
                        current_state = actor.consume_any_message(msg, ctx, current_state);
                        processed += 1;
                    }
                    Err(_) => break,
                }
            }

            self.states.insert(actor_ref, current_state);

            let hit_message_limit = processed >= self.max_batch_messages;
            let hit_time_limit = Instant::now() >= deadline;

            if hit_message_limit || hit_time_limit {
                self.queue_actor_if_pending(actor_ref, receiver);
            } else {
                self.return_to_unqueued(actor_ref, receiver);
            }
        } else {
            self.return_to_unqueued(actor_ref, receiver);
        }
    }

    fn queue_actor_if_pending(&self, actor_ref: ActorRef, receiver: Receiver<ActorMessage>) {
        if let Ok(msg_data) = receiver.try_recv() {
            self.queue.lock().unwrap().push_back((actor_ref, receiver, msg_data));
        } else {
            self.return_to_unqueued(actor_ref, receiver);
        }
    }

    fn return_to_unqueued(&self, actor_ref: ActorRef, receiver: Receiver<ActorMessage>) {
        self.unqueued.insert(actor_ref, Mutex::new(receiver));
    }

    pub fn send(&self, to: ActorRef, from: Option<ActorRef>, msg: Box<dyn Any + Send>) {
        if let Some(sender) = self.senders.get(&to) {
            let _ = sender.send((msg, MsgCtx { from, to }));

            if let Some((_, receiver_mutex)) = self.unqueued.remove(&to) {
                let receiver = receiver_mutex.into_inner().unwrap();
                self.queue_actor_if_pending(to, receiver);
            }
        }
    }

    pub fn add_actor<A>(self: &Arc<Self>, mut actor: A, init_param: A::I) -> ActorRef
where
    A: Actor + StageAware + 'static + Send + Sync,
{
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let actor_ref = ActorRef(id);
        let handle = ActorHandle::new(self, actor_ref);
        actor.set_handle(handle);
        
        let initial_state = actor.handle_init(init_param);
        self.states.insert(actor_ref, Box::new(initial_state));
        self.actors.insert(actor_ref, Arc::new(actor));

        let (tx, rx) = mpsc::channel();
        self.senders.insert(actor_ref, tx);
        self.return_to_unqueued(actor_ref, rx);

        actor_ref
    }
}

#[derive(Clone)]
pub struct ActorHandle {
    stage: Weak<Stage>,
    self_ref: ActorRef,
}

impl ActorHandle {
    pub fn new(stage: &Arc<Stage>, self_ref: ActorRef) -> Self {
        Self {
            stage: Arc::downgrade(stage),
            self_ref,
        }
    }

    pub fn self_ref(&self) -> ActorRef {
        self.self_ref
    }

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
    use std::time::Duration;

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

        fn consume_message(&self, msg: Box<i32>, _ctx: MsgCtx, state: i32) -> i32 {
            state + *msg
        }

        fn handle_init(&self, param: i32) -> i32 {
            param
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

            thread::sleep(Duration::from_millis(5));
        }

        let state = stage.states.get(&actor_ref).unwrap();
        let state = state.downcast_ref::<i32>().unwrap();
        assert_eq!(*state, expected);
    }

    #[test]
    fn test_stage_send_accumulates_messages() {
        let stage = Stage::new(2, 10, Duration::from_millis(10));
        let actor_ref = stage.add_actor(MyActor::new(), 0);

        stage.send(actor_ref, None, Box::new(10));
        stage.send(actor_ref, None, Box::new(20));

        wait_for_state(&stage, actor_ref, 30);
    }

    #[test]
    fn test_stage_batches_more_than_limit() {
        let stage = Stage::new(2, 10, Duration::from_millis(10));
        let actor_ref = stage.add_actor(MyActor::new(), 0);

        for _ in 0..11 {
            stage.send(actor_ref, None, Box::new(1));
        }

        wait_for_state(&stage, actor_ref, 11);
    }

    #[test]
    fn test_multiple_actors_unique_refs() {
        let stage = Stage::new(1, 10, Duration::from_millis(10));
        let ref1 = stage.add_actor(MyActor::new(), 0);
        let ref2 = stage.add_actor(MyActor::new(), 0);

        assert_ne!(ref1, ref2);
    }
}
