# rust-experiments
Some experiments while learning Rust.

## proplist

A basic implementation of Erlang-style property lists. Create an imutable singly linked list with the macro `list![1,2,3]`, or use `arc_list![1, 2, 3]` if you want the values to live on the heap.

A property list is a list of key-value pairs, with some methods similar to these you'd expect on a map. You can create one like this: `list![("a", 1),("b", 2),("a", 3)]`.


## Actrs

An attempt at implementing actors similar to `gen_server` in Erlang. The framework schedules actors over a configurable number of worker threads.

For a full demo, see [actrs_example/src/main.rs](actrs_example/src/main.rs).

### Basic Usage

1. **Create a Stage**: The `Stage` manages actor scheduling and execution.
   ```rust
   let stage = actrs::Stage::new(4, 10, Duration::from_millis(5));
   ```
   This stage will hav 4 worker threads, and actors will keep running until they processed a maximum of 10 messages or ran for 5 milliseconds. After that, the thread will switch to another actor.

2. **Define an Actor**: Use the `#[actor]` and `#[initialize]` macros.
   ```rust
   #[derive(Default)]
   pub struct MyState { count: u32 }

   pub struct CounterActor { _handle: Option<actrs::ActorHandle> }
   pub struct Inc {}
   pub struct Stop {}
      
   #[actor(state = MyState)]
   impl CounterActor {
       #[initialize]
       fn initialize(&self, param: MyState) -> MyState {
           param
       }

       #[message_consumer(Inc)]
       fn handle_inc(&self, _msg: Inc, _ctx: actrs::MsgCtx, state: MyState) -> MyState {
           MyState { count: state.count + 1 }
       }
   
       #[message_consumer(Stop)]
       fn handle_stop(&self, _msg: Stop, _ctx: actrs::MsgCtx, state: MyState) -> actrs::ActorResult<MyState> {
           actrs::ActorResult::Stop
       }
   }
   ```

3. **Spawn and Communicate**:
   ```rust
    let counter_ref = stage.add_actor(CounterActor{ _handle: None }, MyState::default());
    stage.send(counter_ref.clone(), None, Box::new(CounterActorMessage::from(Inc {})));
   ```

Actors stop when they return `ActorResult::Stop` or `ActorResult::Error("reason")`.
If a handle_* function returns the state, that state will automatically be wrapped in an `ActorResult::Ok`.
Wait for completion by checking `stage.actor_count() == 0` and call `stage.shutdown()`.

### Macro

This is what the macro in this example will expand to
```rust
impl CounterActor {
    fn initialize(&self, param: MyState) -> MyState {
        param
    }

    fn handle_inc(&self, _msg: Inc, _ctx: actrs::MsgCtx, state: MyState) -> MyState {
        MyState { count: state.count + 1 }
    }

    fn handle_stop(&self, _msg: Stop, _ctx: actrs::MsgCtx, state: MyState) -> actrs::ActorResult<MyState> {
        actrs::ActorResult::Stop
    }
}
pub enum CounterActorMessage {
    Inc(Inc),
    Stop(Stop),
}
impl ::actrs::Actor for CounterActor {
    type S = MyState;
    type M = CounterActorMessage;
    type I = MyState;
    fn consume_message(&self, msg: Box<Self::M>, ctx: ::actrs::MsgCtx, state: Self::S) -> ::actrs::ActorResult<Self::S> {
        match *msg {
            CounterActorMessage::Inc(inner) => {
                let res = self.handle_inc(inner, ctx, state);   #[allow(
                    unused_imports
                )]
                use ::actrs::ToActorResult;
                res.to_actor_result()
            }
            CounterActorMessage::Stop(inner) => {
                let res = self.handle_stop(inner, ctx, state);   #[allow(
                    unused_imports
                )]
                use ::actrs::ToActorResult;
                res.to_actor_result()
            }
        }
    }
    fn handle_init(&self, param: Self::I) -> ::actrs::ActorResult<Self::S> {
        let res = match param { _ => self.initialize(param), };   #[allow(
            unused_imports
        )]
        use ::actrs::ToActorResult;
        res.to_actor_result()
    }
}
impl ::actrs::StageAware for CounterActor {
    fn set_handle(&mut self, handle: ::actrs::ActorHandle) { self._handle = Some(handle); }
}
impl CounterActor {
    pub fn send<M, TargetActorMessage>(&self, to: ::actrs::ActorRef, msg: M) -> ::core::result::Result<(), &'static str>
    where
        M: ::core::any::Any + Send + 'static,
        TargetActorMessage: ::actrs::Actor + ::core::any::Any + Send + 'static,
        <TargetActorMessage as ::actrs::Actor>::M: ::core::convert::From<M>,
    {
        match &self._handle {
            Some(handle) => handle.send(to, Box::new(<<TargetActorMessage as ::actrs::Actor>::M>::from(msg))),
            None => Err("actor handle not initialized"),
        }
    }
}
impl ::core::convert::From<Inc> for CounterActorMessage {
    fn from(value: Inc) -> Self { CounterActorMessage::Inc(value) }
}
impl ::core::convert::From<Stop> for CounterActorMessage {
    fn from(value: Stop) -> Self { CounterActorMessage::Stop(value) }
}
```

### Todo

Basic functionality seems to be there. This would still be nice:
 * Avoid calling any of the functions directly that take a dyn Any Box. The macro should generate functions to add an actor to the stage with the correct initialization parameter and to send the various accepted messages. So all function calls can be statically typed, and the dyn Any boxes become an implementation detail.
 * Maybe add an Actor registry that allows you to contact actors by name
 * Better error handling. Log the actor state and last message in case of a failure? Create some system event bus where actor lifecyle events are published, which could be used by a system that implements some recovery in case of actor failure.
 * Timers that send messages.
 * More date in the MsgCtx. A session number, that can be used to link replies to the original request. A timestamp of when the messge was sent, to be able to log delays.
 * Do some code cleanup, add comments, documentation and split up some files
 * Find a way to test better with multiple threads to make sure scheduling works correctly, and shared data is protected adequately.
