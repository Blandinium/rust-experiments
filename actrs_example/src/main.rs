
use actrs_macros::actor;
use actrs::{Actor, ActorRef, MsgCtx};

pub mod ping {
    #[derive(Debug)]
    pub struct Ping(pub String);
}

pub struct MyActor {
    _handle: Option<actrs::ActorHandle>,
}

#[actor(state = usize, handle = _handle)]
impl MyActor {
    #[initialize]
    fn init(&self) -> usize {
        42
    }

    #[message_consumer(Ping)]
    fn handle_ping(&self, msg: ping::Ping, ctx: MsgCtx, state: usize) -> usize {
        println!("Ping from {:?}: {:?}", ctx.from, msg.0);
        state+1
    }

    #[message_consumer(Stop)]
    fn handle_stop(&self, ctx: MsgCtx, state: usize) -> usize {
        println!("Stop from {:?}", ctx.from);
        state
    }
}

fn main() {
    let actor = MyActor{ _handle: None };
    let ctx = MsgCtx { from: Some(ActorRef(1)), to: ActorRef(2) };

    let state = actor.init();
    println!("initial state = {}", state);

    let state = actor.consume_message(
        Box::from(MyActorMessage::Ping(ping::Ping("hello".into()))),
        ctx,
        state,
    );

    let state = actor.consume_message(Box::from(MyActorMessage::Stop), ctx, state);

    println!("final state = {}", state);
}
