
use actrs_macros::actor;

#[derive(Debug, Clone, Copy)]
pub struct Pid(pub u64);

pub mod ping {
    #[derive(Debug)]
    pub struct Ping(pub String);
}

pub struct MyActor {
}

#[actor(state = usize)]
impl MyActor {
    #[message_consumer(Ping)]
    fn handle_ping(&self, msg: ping::Ping, from: Pid, state: usize) -> usize {
        println!("Ping from {:?}: {:?}", from.0, msg.0);
        state+1
    }

    #[message_consumer(Stop)]
    fn handle_stop(&self, from: Pid, state: usize) -> usize {
        println!("Stop from {:?}", from.0);
        state
    }
}

fn main() {
    let actor = MyActor{};
    let pid = Pid(42);

    let state = 10usize;

    let state = actor.consume_message(
        MyActorMessage::Ping(ping::Ping("hello".into())),
        pid,
        state,
    );

    let state = actor.consume_message(MyActorMessage::Stop, pid, state);

    let state = actor.consume_message(ping::Ping("via From".into()).into(), pid, state);

    println!("final state = {}", state);
}
